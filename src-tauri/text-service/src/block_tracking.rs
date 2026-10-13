//! Current-block capability, independent of optional language-learning watches.
//! All ranges and subscriptions stay on the owning apartment.
use super::*;
use unicode_segmentation::UnicodeSegmentation;
#[path = "block_boundary.rs"]
mod boundary;
use boundary::BoundaryProof;

pub(super) struct Track {
    pub context: ITfContext,
    range: ITfRange,
    field: initial_selection::FocusIdentity,
    task: String,
    data: Rc<RefCell<Data>>,
    cookie: u32,
    manager_source: ITfSource,
    manager_cookie: u32,
    notifications: Rc<TextNotifications>,
    boundary: Option<Rc<BoundaryProof>>,
    _lease: Lease,
}
struct Data {
    id: String,
    epoch: u64,
    version: u64,
    operation: u64,
    text: String,
    valid: bool,
    invalid_reason: &'static str,
    writable: bool,
    lease: u64,
    writing: bool,
}
impl Drop for Track {
    fn drop(&mut self) {
        self.data.borrow_mut().valid = false;
        self.notifications.retire(self.context.clone(), self.cookie);
        self.notifications
            .retire_source(self.manager_source.clone(), self.manager_cookie);
    }
}

unsafe fn relative_selection(
    context: &ITfContext,
    range: &ITfRange,
    ec: u32,
) -> windows::core::Result<(u32, u32)> {
    unsafe {
        let selected = selection(context, ec)?;
        if selected.CompareStart(ec, range, TF_ANCHOR_START)? < 0
            || selected.CompareEnd(ec, range, TF_ANCHOR_END)? > 0
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let prefix = range.Clone()?;
        prefix.ShiftEndToRange(ec, &selected, TF_ANCHOR_START)?;
        let start = range_text(&prefix, ec)?.encode_utf16().count() as u32;
        prefix.ShiftEndToRange(ec, &selected, TF_ANCHOR_END)?;
        let end = range_text(&prefix, ec)?.encode_utf16().count() as u32;
        Ok((start, end))
    }
}

impl Track {
    unsafe fn bind(
        shared: &Shared,
        context: &ITfContext,
        request: &Request,
        ec: u32,
    ) -> windows::core::Result<Self> {
        unsafe {
            let command = request
                .block
                .as_ref()
                .ok_or_else(|| failure(E_INVALIDARG))?;
            let (composition, expected, client, notifications) = {
                let state = shared.borrow();
                if state.context.as_ref() != Some(context)
                    || !state.version.owned
                    || state.version.task != request.task
                    || state.version.turn != request.turn
                    || state.block.is_some()
                {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                (
                    state
                        .composition
                        .clone()
                        .ok_or_else(|| failure(TF_E_DISCONNECTED))?,
                    state.expected.clone(),
                    state.client,
                    state.text_notifications.clone(),
                )
            };
            let range = composition.GetRange()?.Clone()?;
            if range_text(&range, ec)? != expected || expected.is_empty() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let field = initial_selection::FocusIdentity::capture()?;
            range.SetGravity(ec, TF_GRAVITY_BACKWARD, TF_GRAVITY_FORWARD)?;
            // Unavailable/oversized outside evidence keeps the original strict
            // notification rule. It does not block ordinary composition input.
            let boundary = BoundaryProof::capture(context, &range, ec)
                .ok()
                .map(Rc::new);
            let data = Rc::new(RefCell::new(Data {
                id: command.id.clone(),
                epoch: COMPOSITION_EPOCH.fetch_add(1, Ordering::AcqRel) + 1,
                version: 1,
                operation: request.operation,
                text: expected,
                valid: true,
                invalid_reason: "blockInvalid",
                writable: false,
                lease: 0,
                writing: false,
            }));
            let sink: ITfTextEditSink = Sink {
                field: field.clone(),
                range: range.clone(),
                client,
                data: data.clone(),
                notifications: notifications.clone(),
                boundary: boundary.clone(),
                _lease: Lease::new(),
            }
            .into();
            let source: ITfSource = context.cast()?;
            let manager = shared
                .borrow()
                .manager
                .clone()
                .ok_or_else(|| failure(E_FAIL))?;
            let manager_source: ITfSource = manager.cast()?;
            let focus: ITfThreadMgrEventSink = FocusSink {
                context: context.clone(),
                data: data.clone(),
                notifications: notifications.clone(),
                _lease: Lease::new(),
            }
            .into();
            let cookie = source.AdviseSink(&ITfTextEditSink::IID, &sink)?;
            let manager_cookie =
                match manager_source.AdviseSink(&ITfThreadMgrEventSink::IID, &focus) {
                    Ok(cookie) => cookie,
                    Err(error) => {
                        notifications.retire(context.clone(), cookie);
                        return Err(error);
                    }
                };
            Ok(Self {
                context: context.clone(),
                range,
                field,
                task: request.task.clone(),
                data,
                cookie,
                manager_source,
                manager_cookie,
                notifications,
                boundary,
                _lease: Lease::new(),
            })
        }
    }

    unsafe fn snapshot(
        &self,
        ec: u32,
        phase: &mut &'static str,
    ) -> windows::core::Result<BlockSnapshot> {
        unsafe {
            let version = self.data.borrow().version;
            if !self.data.borrow().valid {
                *phase = self.data.borrow().invalid_reason;
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "blockSnapshotField";
            if !self.field.matches()? {
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "blockSnapshotForeground";
            let foreground = GetForegroundWindow();
            let mut process = 0;
            GetWindowThreadProcessId(foreground, Some(&mut process));
            if process != std::process::id() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "blockSnapshotView";
            let view = self.context.GetActiveView()?.GetWnd()?;
            if !view.0.is_null() && GetAncestor(view, GA_ROOT) != foreground {
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "blockSnapshotAccess";
            ensure_enabled_context(&self.context)?;
            if self.context.GetStatus()?.dwDynamicFlags & TF_SD_READONLY != 0
                || password(&self.context, &self.range, ec)?
            {
                return Err(failure(E_ACCESSDENIED));
            }
            *phase = "blockSnapshotText";
            let text = range_text(&self.range, ec)?;
            *phase = "blockSnapshotSelection";
            let (selection_start, selection_end) =
                relative_selection(&self.context, &self.range, ec)?;
            *phase = "blockSnapshotBoundary";
            if let Some(boundary) = &self.boundary
                && !boundary.matches(&self.context, &self.range, ec)?
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "blockSnapshotField";
            if !self.field.matches()? {
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "blockSnapshotVersion";
            let data = self.data.borrow();
            if !data.valid {
                *phase = data.invalid_reason;
                return Err(failure(TF_E_DISCONNECTED));
            }
            if data.version != version || text.is_empty() || data.text != text {
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "block";
            Ok(BlockSnapshot {
                lease: data.lease,
                id: data.id.clone(),
                epoch: data.epoch,
                version,
                text,
                selection_start,
                selection_end,
                writable: data.writable,
            })
        }
    }
}

#[implement(ITfTextEditSink)]
struct Sink {
    field: initial_selection::FocusIdentity,
    range: ITfRange,
    client: u32,
    data: Rc<RefCell<Data>>,
    notifications: Rc<TextNotifications>,
    boundary: Option<Rc<BoundaryProof>>,
    _lease: Lease,
}
impl ITfTextEditSink_Impl for Sink_Impl {
    fn OnEndEdit(
        &self,
        context: Ref<ITfContext>,
        ec: u32,
        record: Ref<ITfEditRecord>,
    ) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        let _notification = TextNotification::enter(self.notifications.clone());
        if !self.data.borrow().valid {
            return Ok(());
        }
        let mut phase = "blockEditContext";
        let result = (|| unsafe {
            let context = context.as_ref().ok_or_else(|| failure(E_POINTER))?;
            // Own composition updates remain subject to native text proof; they
            // update tracking but must never become user-edit learning evidence.
            let own = context.InWriteSession(self.client)?.as_bool();
            phase = "blockEditField";
            if !self.field.matches()? {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let record = record.as_ref().ok_or_else(|| failure(E_POINTER))?;
            phase = "blockEditUpdates";
            let within = updates_within(record, &self.range, ec)?;
            #[cfg(feature = "acceptance")]
            let within = {
                let coarse = block_faults::coarse_notification();
                within && !coarse
            };
            phase = "blockEditOutside";
            if !within
                && !self.boundary.as_ref().is_some_and(|boundary| {
                    boundary.matches(context, &self.range, ec).unwrap_or(false)
                })
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            phase = "blockEditSelection";
            relative_selection(context, &self.range, ec)?;
            phase = "blockEditText";
            let text = range_text(&self.range, ec)?;
            if text.is_empty() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let changed = record.GetSelectionStatus()?.as_bool();
            Ok((own, text, changed))
        })();
        let mut data = self.data.borrow_mut();
        match result {
            Ok((own, text, selection_changed)) => {
                let text_changed = data.text != text;
                if data.text != text {
                    data.text = text;
                    data.version += 1;
                }
                if !own && (text_changed || selection_changed) {
                    data.writable = false;
                    data.lease += 1;
                }
            }
            Err(_) => {
                if data.valid {
                    data.invalid_reason = phase;
                }
                data.valid = false;
                data.writable = false;
            }
        }
        // Tracking refusal must not fail the user's edit.
        Ok(())
    }
}

pub(super) fn release(shared: &Shared) {
    let old = shared.borrow_mut().block.take();
    drop(old);
}
pub(super) fn writing(shared: &Shared) -> bool {
    shared
        .borrow()
        .block
        .as_ref()
        .is_some_and(|t| t.data.borrow().writing)
}
pub(super) fn revoke(shared: &Shared) {
    if let Some(track) = shared.borrow().block.as_ref() {
        let mut data = track.data.borrow_mut();
        data.lease += 1;
        data.writable = false;
    }
}
pub(super) fn invalidate(shared: &Shared, reason: &'static str) {
    if let Some(track) = shared.borrow().block.as_ref() {
        let mut data = track.data.borrow_mut();
        if data.valid {
            data.invalid_reason = reason;
        }
        data.valid = false;
        data.writable = false;
        data.lease += 1;
    }
}

#[implement(ITfThreadMgrEventSink)]
struct FocusSink {
    context: ITfContext,
    data: Rc<RefCell<Data>>,
    notifications: Rc<TextNotifications>,
    _lease: Lease,
}
impl FocusSink {
    fn invalidate(&self, reason: &'static str) {
        let mut data = self.data.borrow_mut();
        if data.valid {
            data.invalid_reason = reason;
        }
        data.valid = false;
        data.writable = false;
        data.lease += 1;
    }
}
impl ITfThreadMgrEventSink_Impl for FocusSink_Impl {
    fn OnInitDocumentMgr(&self, _: Ref<ITfDocumentMgr>) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnUninitDocumentMgr(&self, document: Ref<ITfDocumentMgr>) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        let _notification = TextNotification::enter(self.notifications.clone());
        if unsafe { self.context.GetDocumentMgr() }.ok().as_ref() == document.as_ref() {
            self.invalidate("blockDocumentRetired");
        }
        Ok(())
    }
    fn OnSetFocus(
        &self,
        document: Ref<ITfDocumentMgr>,
        _: Ref<ITfDocumentMgr>,
    ) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        let _notification = TextNotification::enter(self.notifications.clone());
        if document
            .as_ref()
            .and_then(|d| unsafe { d.GetTop() }.ok())
            .as_ref()
            != Some(&self.context)
        {
            self.invalidate("blockDocumentFocus");
        }
        Ok(())
    }
    fn OnPushContext(&self, context: Ref<ITfContext>) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        let _notification = TextNotification::enter(self.notifications.clone());
        if let Some(context) = context.as_ref()
            && context != &self.context
        {
            let own = unsafe { self.context.GetDocumentMgr() };
            let added = unsafe { context.GetDocumentMgr() };
            if own.is_err() || own.ok() == added.ok() {
                self.invalidate("blockContextPushed");
            }
        }
        Ok(())
    }
    fn OnPopContext(&self, context: Ref<ITfContext>) -> windows::core::Result<()> {
        if context.as_ref() == Some(&self.context) {
            self.invalidate("blockContextPopped");
        }
        Ok(())
    }
}

pub(super) fn authorized(
    shared: &Shared,
    request: &Request,
    connection: u64,
) -> windows::core::Result<()> {
    let state = shared.borrow();
    if !state.active
        || !state.connected
        || state.connection != connection
        || state.capability.as_ref() != Some(&request.capability)
        || state.binding != request.engine
        || !request.live(unsafe { GetTickCount64() })
        || state.cleanup
        || state.hosted && !hosted_channel_alive(&state, &request.capability)
    {
        return Err(failure(TF_E_DISCONNECTED));
    }
    let peer = state.peer.ok_or_else(|| failure(E_ACCESSDENIED))?;
    if state
        .pipe
        .as_ref()
        .is_some_and(|p| !p.alive() || p.client_pid().ok() != Some(peer.pid))
    {
        return Err(failure(TF_E_DISCONNECTED));
    }
    drop(state);
    if registration::read_host() != Some((peer.pid, request.capability.clone()))
        || identity(peer.pid).ok() != Some(peer)
    {
        return Err(failure(TF_E_DISCONNECTED));
    }
    Ok(())
}
fn edit_authorized(
    shared: &Shared,
    r: &Request,
    connection: u64,
    generation: u64,
) -> windows::core::Result<()> {
    authorized(shared, r, connection)?;
    let state = shared.borrow();
    if state.generation != generation
        || !state.pending.as_ref().is_some_and(|p| {
            p.task == r.task
                && p.turn == r.turn
                && p.operation == r.operation
                && p.revision == r.revision
                && p.connection == connection
                && p.capability == r.capability
        })
    {
        return Err(failure(TF_E_DISCONNECTED));
    }
    Ok(())
}

pub(super) fn dispatch(shared: &Shared, request: &Request, connection: u64) {
    let result = (|| unsafe {
        authorized(shared, request, connection)?;
        if shared.borrow().pending.is_some() {
            return Err(failure(TF_E_LOCKED));
        }
        let command = request
            .block
            .as_ref()
            .ok_or_else(|| failure(E_INVALIDARG))?;
        if command.action == BlockAction::Release {
            let matched = shared.borrow().block.as_ref().is_some_and(|track| {
                let data = track.data.borrow();
                track.task == request.task
                    && data.id == command.id
                    && data.epoch == command.epoch
                    && request.operation > data.operation
            });
            if !matched {
                return Err(failure(TF_E_DISCONNECTED));
            }
            release(shared);
            reply(
                shared,
                request,
                connection,
                true,
                "blockReleased",
                Mutation::NotAttempted,
            );
            return Ok(());
        }
        let (context, manager, client) = {
            let state = shared.borrow();
            let context = if command.action == BlockAction::Bind {
                state.context.clone()
            } else {
                state.block.as_ref().map(|track| track.context.clone())
            }
            .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
            (
                context,
                state.manager.clone().ok_or_else(|| failure(E_FAIL))?,
                state.client,
            )
        };
        if manager.GetFocus()?.GetTop()? != context {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let edit: ITfEditSession = EditBlock {
            generation: shared.borrow().generation,
            shared: shared.clone(),
            request: request.clone(),
            connection,
            context: context.clone(),
            _lease: Lease::new(),
        }
        .into();
        shared.borrow_mut().pending = Some(Pending {
            request: Some(request.clone()),
            started: false,
            operation: request.operation,
            revision: request.revision,
            task: request.task.clone(),
            turn: request.turn,
            connection,
            capability: request.capability.clone(),
            settlement: None,
        });
        let flags = TF_ES_ASYNCDONTCARE
            | if command.action == BlockAction::Read {
                TF_ES_READ
            } else {
                TF_ES_READWRITE
            };
        let result = request_native_edit(&context, client, &edit, flags)?;
        if shared
            .borrow()
            .pending
            .as_ref()
            .is_some_and(|p| p.operation == request.operation)
        {
            result.ok()?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        reply_outcome(
            shared,
            request,
            connection,
            false,
            &format!("block:{:08x}", error.code().0 as u32),
            Mutation::NotAttempted,
            None,
        );
    }
}

#[implement(ITfEditSession)]
struct EditBlock {
    generation: u64,
    shared: Shared,
    context: ITfContext,
    request: Request,
    connection: u64,
    _lease: Lease,
}
impl ITfEditSession_Impl for EditBlock_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        let mut mutation = Mutation::NotAttempted;
        let mut phase = "block";
        let result = (|| unsafe {
            let shared = &self.shared;
            let r = &self.request;
            edit_authorized(shared, r, self.connection, self.generation)?;
            let command = r.block.as_ref().ok_or_else(|| failure(E_INVALIDARG))?;
            let manager = shared
                .borrow()
                .manager
                .clone()
                .ok_or_else(|| failure(E_FAIL))?;
            if manager.GetFocus()?.GetTop()? != self.context {
                return Err(failure(TF_E_DISCONNECTED));
            }
            if command.action == BlockAction::Bind {
                let epoch = shared.borrow().epoch;
                let track = Rc::new(Track::bind(shared, &self.context, r, ec)?);
                let snapshot = track.snapshot(ec, &mut phase)?;
                edit_authorized(shared, r, self.connection, self.generation)?;
                let mut state = shared.borrow_mut();
                if state.epoch != epoch
                    || !state.version.owned
                    || state.version.task != r.task
                    || state.version.turn != r.turn
                    || state.block.is_some()
                    || state.expected != snapshot.text
                {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                state.block = Some(track);
                return Ok(snapshot);
            }
            let track = shared
                .borrow()
                .block
                .clone()
                .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
            let snapshot = track.snapshot(ec, &mut phase)?;
            if track.task != r.task
                || snapshot.id != command.id
                || snapshot.epoch != command.epoch
                || r.operation <= track.data.borrow().operation
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            if matches!(command.action, BlockAction::Acquire | BlockAction::Apply) {
                if snapshot.version != command.version
                    || snapshot.text != command.expected
                    || snapshot.selection_start != command.selection_start
                    || snapshot.selection_end != command.selection_end
                    || shared.borrow().version.owned
                    || shared.borrow().composition.is_some()
                {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                let owner: ITfContextComposition = self.context.cast()?;
                let mut compositions = [None];
                let mut fetched = 0;
                owner
                    .EnumCompositions()?
                    .Next(&mut compositions, &mut fetched)?;
                if fetched != 0 {
                    return Err(failure(TF_E_LOCKED));
                }
            }
            edit_authorized(shared, r, self.connection, self.generation)?;
            if shared
                .borrow()
                .block
                .as_ref()
                .is_none_or(|current| !Rc::ptr_eq(current, &track))
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            track.data.borrow_mut().operation = r.operation;
            match command.action {
                BlockAction::Acquire => {
                    let mut data = track.data.borrow_mut();
                    data.lease += 1;
                    data.writable = true;
                }
                BlockAction::Apply => {
                    if !track.data.borrow().writable
                        || snapshot.lease != command.lease
                        || r.text.is_empty()
                    {
                        return Err(failure(TF_E_DISCONNECTED));
                    }
                    // Only map endpoints outside the changed interval. An
                    // ambiguous caret refuses the replacement before SetText.
                    phase = "blockSelectionMapping";
                    let end = map_endpoint(&snapshot.text, &r.text, snapshot.selection_end)
                        .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
                    phase = "block";
                    let before = track.snapshot(ec, &mut phase)?;
                    if before != snapshot {
                        return Err(failure(TF_E_DISCONNECTED));
                    }
                    edit_authorized(shared, r, self.connection, self.generation)?;
                    start_effect(shared, r)?;
                    track.data.borrow_mut().writing = true;
                    mutation = Mutation::Uncertain;
                    let write = track.range.SetText(
                        ec,
                        TF_ST_CORRECTION,
                        &r.text.encode_utf16().collect::<Vec<_>>(),
                    );
                    track.data.borrow_mut().writing = false;
                    write?;
                    edit_authorized(shared, r, self.connection, self.generation)?;
                    let text = range_text(&track.range, ec)?;
                    if !equivalent_native_text(&text, &r.text) {
                        return Err(failure(TF_E_DISCONNECTED));
                    }
                    let end = equivalent_position(&r.text, &text, end)
                        .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
                    let caret = track.range.Clone()?;
                    caret.Collapse(ec, TF_ANCHOR_START)?;
                    let mut moved = 0;
                    caret.ShiftEnd(ec, end as i32, &mut moved, std::ptr::null())?;
                    if moved != end as i32 {
                        return Err(failure(TF_E_DISCONNECTED));
                    }
                    caret.ShiftStart(ec, end as i32, &mut moved, std::ptr::null())?;
                    if moved != end as i32 {
                        return Err(failure(TF_E_DISCONNECTED));
                    }
                    edit_authorized(shared, r, self.connection, self.generation)?;
                    let mut selected = TF_SELECTION {
                        range: ManuallyDrop::new(Some(caret)),
                        style: TF_SELECTIONSTYLE {
                            ase: TF_AE_NONE,
                            fInterimChar: false.into(),
                        },
                    };
                    let result = self
                        .context
                        .SetSelection(ec, std::slice::from_ref(&selected));
                    ManuallyDrop::drop(&mut selected.range);
                    result?;
                    edit_authorized(shared, r, self.connection, self.generation)?;
                    mutation = Mutation::Changed;
                    let mut data = track.data.borrow_mut();
                    data.text = text;
                    data.version += 1;
                }
                BlockAction::Read => (),
                _ => return Err(failure(E_INVALIDARG)),
            }
            track.snapshot(ec, &mut phase)
        })();
        match &result {
            Ok(snapshot) => {
                self.shared.borrow_mut().block_reply = Some(snapshot.clone());
                complete(
                    &self.shared,
                    &self.request,
                    self.connection,
                    true,
                    "blockReady",
                    mutation,
                    None,
                );
            }
            Err(error) => complete(
                &self.shared,
                &self.request,
                self.connection,
                false,
                &format!("{phase}:{:08x}", error.code().0 as u32),
                mutation,
                None,
            ),
        }
        result.map(|_| ())
    }
}

fn map_endpoint(before: &str, after: &str, at: u32) -> Option<u32> {
    fn boundary(text: &str, at: u32) -> bool {
        let mut units = 0;
        for part in text.graphemes(true) {
            if units == at {
                return true;
            }
            units += part.encode_utf16().count() as u32;
        }
        units == at
    }
    if !boundary(before, at) {
        return None;
    }
    let a: Vec<char> = before.chars().collect();
    let b: Vec<char> = after.chars().collect();
    let mut prefix = 0;
    while prefix < a.len().min(b.len()) && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < a.len() - prefix
        && suffix < b.len() - prefix
        && a[a.len() - suffix - 1] == b[b.len() - suffix - 1]
    {
        suffix += 1;
    }
    let units = |chars: &[char]| chars.iter().map(|c| c.len_utf16() as u32).sum::<u32>();
    let total = units(&a);
    if at > total || !(0..=a.len()).any(|i| units(&a[..i]) == at) {
        return None;
    }
    if at == total {
        return Some(units(&b));
    }
    let mapped = if at >= units(&a[..a.len() - suffix]) {
        units(&b) - (total - at)
    } else if at <= units(&a[..prefix]) {
        at
    } else {
        return None;
    };
    boundary(after, mapped).then_some(mapped)
}
fn equivalent_position(requested: &str, actual: &str, at: u32) -> Option<u32> {
    fn tokens(text: &str) -> Vec<(char, u32)> {
        let mut chars = text.chars().peekable();
        let mut end = 0;
        let mut result = vec![];
        while let Some(c) = chars.next() {
            end += c.len_utf16() as u32;
            if c == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                    end += 1;
                }
                result.push(('\n', end));
            } else {
                result.push((c, end));
            }
        }
        result
    }
    let from = tokens(requested);
    let to = tokens(actual);
    if from.iter().map(|p| p.0).ne(to.iter().map(|p| p.0)) {
        return None;
    }
    if at == 0 {
        return Some(0);
    }
    let index = from.iter().position(|p| p.1 == at)?;
    Some(to[index].1)
}
