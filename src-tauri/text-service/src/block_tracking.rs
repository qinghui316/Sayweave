//! Current-block capability, independent of optional language-learning watches.
//! All ranges and subscriptions stay on the owning apartment.
use super::*;
use unicode_segmentation::UnicodeSegmentation;
#[path = "block_boundary.rs"]
mod boundary;
use boundary::BoundaryProof;
#[path = "block_content.rs"]
mod content;
#[path = "block_evidence.rs"]
mod evidence;

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
    position: RangePosition,
    valid: bool,
    invalid_reason: String,
    writable: bool,
    lease: u64,
    writing: bool,
    evidence: evidence::Record,
    semantic_boundary: Option<initial_selection::SemanticBoundary>,
    pending_proof: bool,
    reacquired: bool,
    notification: u64,
    notification_depth: u32,
}
struct EditNotification {
    data: Rc<RefCell<Data>>,
    serial: u64,
    version: u64,
    lease: Cell<u64>,
}
impl EditNotification {
    fn enter(data: &Rc<RefCell<Data>>) -> Self {
        let mut state = data.borrow_mut();
        state.notification += 1;
        state.notification_depth += 1;
        Self {
            data: data.clone(),
            serial: state.notification,
            version: state.version,
            lease: Cell::new(state.lease),
        }
    }
    fn current(&self) -> bool {
        let state = self.data.borrow();
        state.valid
            && state.notification == self.serial
            && state.version == self.version
            && state.lease == self.lease.get()
    }
    fn commit_outside(&self) -> bool {
        if !self.current() {
            return false;
        }
        // Called only after the boundary has finished all COM reads. This
        // lease revocation and the outside evidence commit have no reentry
        // point between them; a later superseding notification cannot undo it.
        let mut state = self.data.borrow_mut();
        state.writable = false;
        state.lease += 1;
        self.lease.set(state.lease);
        true
    }
}
impl Drop for EditNotification {
    fn drop(&mut self) {
        self.data.borrow_mut().notification_depth -= 1;
    }
}
impl Drop for Track {
    fn drop(&mut self) {
        self.data.borrow_mut().valid = false;
        self.notifications.retire(self.context.clone(), self.cookie);
        self.notifications
            .retire_source(self.manager_source.clone(), self.manager_cookie);
    }
}

// The composition and the independent tracking clone are different native
// ranges. Refresh from the proven writer, never from offsets or a text search.
pub(super) struct OwnedUpdate {
    track: Rc<Track>,
    version: u64,
    armed: std::cell::Cell<bool>,
}
impl Drop for OwnedUpdate {
    fn drop(&mut self) {
        if self.armed.get() {
            self.track.data.borrow_mut().writing = false;
        }
    }
}
pub(super) unsafe fn owned_update(
    shared: &Shared,
    context: &ITfContext,
    range: &ITfRange,
    ec: u32,
) -> windows::core::Result<Option<OwnedUpdate>> {
    let track = {
        let state = shared.borrow();
        let Some(track) = state.block.clone() else {
            return Ok(None);
        };
        if state.context.as_ref() != Some(context)
            || state.composition.is_none()
            || state.version.task != track.task
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        track
    };
    if !track.data.borrow().valid || track.data.borrow().writing {
        return Err(failure(TF_E_DISCONNECTED));
    }
    let matched = (|| unsafe {
        let mut phase = "blockOwnedBaseline".into();
        if let Some(boundary) = &track.boundary
            && !boundary.matches(context, &track.range, ec, &mut phase)?
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let raw = track.data.borrow().text.clone();
        if track.range.CompareStart(ec, range, TF_ANCHOR_START)? != 0
            || track.range.CompareEnd(ec, range, TF_ANCHOR_END)? != 0
            || range_text(range, ec)? != raw
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        Ok::<_, windows::core::Error>(())
    })();
    let mut data = track.data.borrow_mut();
    if matched.is_err() || !data.valid {
        if data.valid {
            data.invalid_reason = "blockOwnedBaseline".into();
        }
        data.valid = false;
        data.writable = false;
        return Err(failure(TF_E_DISCONNECTED));
    }
    let version = data.version;
    drop(data);
    Ok(Some(OwnedUpdate {
        track,
        version,
        armed: std::cell::Cell::new(false),
    }))
}
impl OwnedUpdate {
    pub(super) fn arm(&self) -> bool {
        let mut data = self.track.data.borrow_mut();
        if !data.valid || data.version != self.version || data.writing {
            return false;
        }
        data.writing = true;
        data.writable = false;
        data.lease += 1;
        self.armed.set(true);
        true
    }
    #[cfg(feature = "acceptance")]
    pub(super) unsafe fn stale_fixture(&self, ec: u32) -> windows::core::Result<()> {
        unsafe {
            if block_faults::stale_tracking_range() {
                self.track.range.Collapse(ec, TF_ANCHOR_START)?;
            }
            Ok(())
        }
    }
    pub(super) unsafe fn follow(
        &self,
        context: &ITfContext,
        range: &ITfRange,
        ec: u32,
        authority: impl Fn() -> windows::core::Result<()>,
    ) {
        let result = (|| unsafe {
            authority()?;
            let valid = self.track.data.borrow().valid;
            if !valid || !self.track.field.matches()? {
                return Err(failure(TF_E_DISCONNECTED));
            }
            if let Some(boundary) = &self.track.boundary {
                boundary.follow_owned_write(context, range, ec)?;
            }
            authority()?;
            self.track
                .range
                .ShiftStartToRange(ec, range, TF_ANCHOR_START)?;
            self.track.range.ShiftEndToRange(ec, range, TF_ANCHOR_END)?;
            let text = range_text(&self.track.range, ec)?;
            relative_selection(context, &self.track.range, ec)?;
            authority()?;
            if !self.track.field.matches()? {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let mut data = self.track.data.borrow_mut();
            if !data.valid {
                return Err(failure(TF_E_DISCONNECTED));
            }
            if data.text != text {
                data.text = text;
                data.version += 1;
            }
            drop(data);
            let position = range_position(context, &self.track.range, ec)?;
            self.track.data.borrow_mut().position = position;
            Ok::<_, windows::core::Error>(())
        })();
        if result.is_err() {
            let mut data = self.track.data.borrow_mut();
            if data.valid {
                data.invalid_reason = "blockOwnedFollow".into();
            }
            data.valid = false;
            data.writable = false;
        }
        let error = result.as_ref().err().map(|e| e.code());
        unsafe {
            evidence::capture(
                context,
                &self.track.range,
                &self.track.field,
                &self.track.data,
                ec,
                ("ownedFollow", "blockOwnedFollow", error),
            );
        }
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
            let mut boundary_reason = "blockBoundaryUnavailable".into();
            let boundary = BoundaryProof::capture(context, &range, ec, &mut boundary_reason)
                .ok()
                .map(Rc::new);
            let data = Rc::new(RefCell::new(Data {
                id: command.id.clone(),
                epoch: COMPOSITION_EPOCH.fetch_add(1, Ordering::AcqRel) + 1,
                version: 1,
                operation: request.operation,
                text: expected,
                position: range_position(context, &range, ec)?,
                valid: true,
                invalid_reason: "blockInvalid".into(),
                writable: false,
                lease: 0,
                writing: false,
                evidence: evidence::Record::default(),
                semantic_boundary: None,
                pending_proof: false,
                reacquired: false,
                notification: 0,
                notification_depth: 0,
            }));
            let sink: ITfTextEditSink = Sink {
                field: field.clone(),
                range: range.clone(),
                client,
                data: data.clone(),
                notifications: notifications.clone(),
                boundary: boundary.clone(),
                boundary_reason,
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
            let track = Self {
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
            };
            // Capture outside semantic evidence only while the original owned
            // range is known. A later failed read cannot invent this baseline.
            if let Ok((_, boundary)) =
                content::capture(context, &track.range, &track.field, None, ec, 1)
            {
                track.data.borrow_mut().semantic_boundary = Some(boundary);
            }
            evidence::capture(
                context,
                &track.range,
                &track.field,
                &track.data,
                ec,
                ("bind", "block", None),
            );
            Ok(track)
        }
    }

    unsafe fn verified_content(
        &self,
        ec: u32,
        phase: &mut String,
    ) -> windows::core::Result<content::VerifiedBlockContent> {
        unsafe {
            let (version, original) = {
                let data = self.data.borrow();
                (data.version, data.semantic_boundary.clone())
            };
            if let Some(boundary) = &self.boundary
                && !boundary.anchors_match(&self.context, &self.range, ec, phase)?
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            // Native scope and edit continuity, not whole-field UIA equality,
            // authorize this exact inline text. The outside suffix is never
            // read into the baseline. Paragraph/object representations require
            // the separate semantic proof below; no character is trimmed.
            let native = range_text(&self.range, ec)?;
            let position = range_position(&self.context, &self.range, ec)?;
            let continuous = {
                let data = self.data.borrow();
                data.valid
                    && data.notification_depth == 0
                    && data.version == version
                    && data.text == native
                    && data.position == position
            };
            if !continuous {
                *phase = "blockSnapshotChanged".into();
                return Err(failure(TF_E_LOCKED));
            }
            if content::inline_identity(&native)
                && let Some(boundary) = &self.boundary
            {
                if !boundary.matches(&self.context, &self.range, ec, phase)? {
                    return Err(failure(TF_E_LOCKED));
                }
                let (selection_start, selection_end) =
                    relative_selection(&self.context, &self.range, ec)?;
                return Ok(content::VerifiedBlockContent {
                    semantic_text: native.clone(),
                    native_text: native,
                    native_end: position.end,
                    selection_start,
                    selection_end,
                    content_version: version,
                });
            }
            if let Some(original) = original {
                // UIA has no revision shared with TSF. Semantic agreement
                // cannot replace original native outside fingerprints.
                if let Some(boundary) = &self.boundary
                    && !boundary.matches(&self.context, &self.range, ec, phase)?
                {
                    return Err(failure(TF_E_LOCKED));
                }
                *phase = "blockAwaitingRepresentation".into();
                let (verified, _) = content::capture(
                    &self.context,
                    &self.range,
                    &self.field,
                    Some(&original),
                    ec,
                    version,
                )
                .inspect_err(|error| {
                    if error.code() == TF_E_DISCONNECTED {
                        *phase = "blockScopeInvalid".into();
                        let mut data = self.data.borrow_mut();
                        data.valid = false;
                        data.writable = false;
                        data.invalid_reason.clone_from(phase);
                    }
                })?;
                if self.data.borrow().version != version || !self.data.borrow().valid {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                return Ok(verified);
            }
            // Native controls without a semantic provider retain strict raw
            // boundaries. Missing semantic evidence never relaxes a mismatch.
            if !self.field.native_identity_text() {
                *phase = "blockAwaitingRepresentation".into();
                return Err(failure(TF_E_LOCKED));
            }
            if let Some(boundary) = &self.boundary
                && !boundary.matches(&self.context, &self.range, ec, phase)?
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let native_text = range_text(&self.range, ec)?;
            let (selection_start, selection_end) =
                relative_selection(&self.context, &self.range, ec)?;
            Ok(content::VerifiedBlockContent {
                semantic_text: native_text.clone(),
                native_text,
                native_end: range_position(&self.context, &self.range, ec)?.end,
                selection_start,
                selection_end,
                content_version: version,
            })
        }
    }

    unsafe fn snapshot(&self, ec: u32, phase: &mut String) -> windows::core::Result<BlockSnapshot> {
        unsafe {
            let version = self.data.borrow().version;
            if !self.data.borrow().valid {
                *phase = self.data.borrow().invalid_reason.clone();
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "blockSnapshotField".into();
            if !self.field.matches()? {
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "blockSnapshotForeground".into();
            let foreground = GetForegroundWindow();
            let mut process = 0;
            GetWindowThreadProcessId(foreground, Some(&mut process));
            if process != std::process::id() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "blockSnapshotView".into();
            let view = self.context.GetActiveView()?.GetWnd()?;
            if !view.0.is_null() && GetAncestor(view, GA_ROOT) != foreground {
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "blockSnapshotAccess".into();
            ensure_enabled_context(&self.context)?;
            if self.context.GetStatus()?.dwDynamicFlags & TF_SD_READONLY != 0
                || password(&self.context, &self.range, ec)?
            {
                return Err(failure(E_ACCESSDENIED));
            }
            let verified = self.verified_content(ec, phase)?;
            let text = verified.native_text;
            let selection_start = verified.selection_start;
            let selection_end = verified.selection_end;
            *phase = "blockSnapshotField".into();
            if !self.field.matches()? {
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "blockSnapshotVersion".into();
            let mut data = self.data.borrow_mut();
            if !data.valid {
                *phase = data.invalid_reason.clone();
                return Err(failure(TF_E_DISCONNECTED));
            }
            if data.version != version || verified.semantic_text.is_empty() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            if data.text != text {
                data.text = text;
                data.version += 1;
                data.writable = false;
                data.lease += 1;
            }
            data.pending_proof = false;
            *phase = "block".into();
            Ok(BlockSnapshot {
                lease: data.lease,
                id: data.id.clone(),
                epoch: data.epoch,
                version: data.version,
                text: verified.semantic_text,
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
    boundary_reason: String,
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
        let writing = self.data.borrow().writing;
        if writing {
            // Intermediate geometry is not a user edit. The writer's own
            // guard and final readback remain authoritative; focus/context
            // sinks still invalidate the capability during this interval.
            return Ok(());
        }
        let notification = EditNotification::enter(&self.data);
        let mut phase = "blockEditContext".into();
        let result = (|| unsafe {
            let context = context.as_ref().ok_or_else(|| failure(E_POINTER))?;
            // Own composition updates remain subject to native text proof; they
            // update tracking but must never become user-edit learning evidence.
            let own = context.InWriteSession(self.client)?.as_bool();
            phase = "blockEditField".into();
            if !self.field.matches()? {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let record = record.as_ref().ok_or_else(|| failure(E_POINTER))?;
            let (previous, previous_position) = {
                let data = self.data.borrow();
                (data.text.clone(), data.position)
            };
            let text = range_text(&self.range, ec)?;
            let position = range_position(context, &self.range, ec)?;
            phase = "blockEditRangeDrift".into();
            if position.start != previous_position.start
                || position.end - position.start != text.encode_utf16().count() as i32
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            phase = "blockEditSelection".into();
            relative_selection(context, &self.range, ec)?;
            phase = "blockEditUpdates".into();
            let within = updates_within(record, &self.range, ec)?;
            #[cfg(feature = "acceptance")]
            let within = {
                let previous = self.data.borrow().text.clone();
                let changed = range_text(&self.range, ec)? != previous;
                let coarse = block_faults::coarse_notification(changed);
                within && !coarse
            };
            let mut disjoint = false;
            if !within {
                phase = self.boundary_reason.clone();
                let boundary = self
                    .boundary
                    .as_ref()
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
                if !boundary.matches(context, &self.range, ec, &mut phase)? {
                    disjoint = text == previous
                        && position == previous_position
                        && boundary.observe_disjoint_suffix(
                            context,
                            &self.range,
                            record,
                            ec,
                            &mut phase,
                            || notification.commit_outside(),
                        )?;
                } else {
                    // Coarse notifications are covered by unchanged outside
                    // bytes; this does not adopt a changed outside snapshot.
                    disjoint = false;
                }
                if !disjoint && !boundary.matches(context, &self.range, ec, &mut phase)? {
                    return Err(failure(TF_E_DISCONNECTED));
                }
            }
            phase = "blockEditSelection".into();
            relative_selection(context, &self.range, ec)?;
            phase = "blockEditTextRead".into();
            if text.is_empty() {
                phase = "blockEditEmpty".into();
                return Err(failure(TF_E_DISCONNECTED));
            }
            phase = "blockEditSelectionStatus".into();
            let changed = record.GetSelectionStatus()?.as_bool();
            Ok((own, text, changed, position, disjoint))
        })();
        let error = result.as_ref().err().map(|e| e.code());
        if !notification.current() {
            // A nested edit/focus callback owns the newer truth. Never revive
            // an invalid range or publish the outer callback's stale sample.
            return Ok(());
        }
        let mut data = self.data.borrow_mut();
        match result {
            Ok((own, text, selection_changed, position, disjoint)) => {
                data.position = position;
                let text_changed = data.text != text;
                if data.text != text {
                    data.text = text;
                    data.version += 1;
                }
                if disjoint || !own && (text_changed || selection_changed) {
                    data.writable = false;
                    data.lease += 1;
                }
            }
            Err(_) => {
                if data.valid {
                    data.invalid_reason = phase.clone();
                }
                let representation_pending = data.semantic_boundary.is_some()
                    && (phase.starts_with("blockSuffixText:")
                        || phase.starts_with("blockPrefixText:"));
                data.pending_proof = representation_pending;
                data.valid = representation_pending;
                data.writable = false;
                data.lease += 1;
            }
        }
        drop(data);
        if let Some(context) = context.as_ref() {
            unsafe {
                evidence::capture(
                    context,
                    &self.range,
                    &self.field,
                    &self.data,
                    ec,
                    ("onEndEdit", &phase, error),
                );
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
            data.invalid_reason = reason.into();
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
            data.invalid_reason = reason.into();
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

#[implement(ITfCompositionSink)]
struct AcquisitionSink {
    delegate: ITfCompositionSink,
    attached: Rc<Cell<bool>>,
    terminated: Rc<Cell<bool>>,
    _lease: Lease,
}

#[cfg(test)]
mod acquisition_tests {
    use super::*;
    fn tracked_state() -> Rc<RefCell<Data>> {
        Rc::new(RefCell::new(Data {
            id: "owned-test".into(),
            epoch: 1,
            version: 1,
            operation: 1,
            text: "字".into(),
            position: RangePosition { start: 0, end: 1 },
            valid: true,
            invalid_reason: "block".into(),
            writable: true,
            lease: 5,
            writing: false,
            evidence: evidence::Record::default(),
            semantic_boundary: None,
            pending_proof: false,
            reacquired: true,
            notification: 0,
            notification_depth: 0,
        }))
    }
    #[test]
    fn outside_commit_revokes_lease_before_a_nested_notification_supersedes_it() {
        let data = tracked_state();
        let outer = EditNotification::enter(&data);
        assert!(outer.commit_outside());
        assert_eq!(data.borrow().lease, 6);
        assert!(!data.borrow().writable);
        assert!(outer.current());
        let nested = EditNotification::enter(&data);
        assert!(!outer.current());
        drop(nested);
        drop(outer);
        assert_eq!(data.borrow().notification_depth, 0);
        assert_eq!(data.borrow().lease, 6);
        assert!(!data.borrow().writable);
    }
    #[test]
    fn nested_invalidation_cannot_commit_an_outer_boundary_sample() {
        let data = tracked_state();
        let outer = EditNotification::enter(&data);
        data.borrow_mut().valid = false;
        assert!(!outer.commit_outside());
        assert!(!outer.current());
        drop(outer);
        assert!(!data.borrow().valid);
        assert_eq!(data.borrow().notification_depth, 0);
    }
    #[implement(ITfCompositionSink)]
    struct Delegate(Rc<Cell<u32>>);
    impl ITfCompositionSink_Impl for Delegate_Impl {
        fn OnCompositionTerminated(
            &self,
            _: u32,
            _: Ref<ITfComposition>,
        ) -> windows::core::Result<()> {
            self.0.set(self.0.get() + 1);
            Ok(())
        }
    }
    #[test]
    fn synchronous_termination_is_retained_before_publication() {
        let callbacks = Rc::new(Cell::new(0));
        let attached = Rc::new(Cell::new(false));
        let terminated = Rc::new(Cell::new(false));
        let sink: ITfCompositionSink = AcquisitionSink {
            delegate: Delegate(callbacks.clone()).into(),
            attached: attached.clone(),
            terminated: terminated.clone(),
            _lease: Lease::new(),
        }
        .into();
        unsafe {
            sink.OnCompositionTerminated(0, None).unwrap();
        }
        assert!(terminated.get());
        assert_eq!(callbacks.get(), 0);
        // Attaching a failed candidate for retirement also enables terminal
        // notifications; this flag never grants a write lease.
        attached.set(true);
        unsafe {
            sink.OnCompositionTerminated(0, None).unwrap();
        }
        assert_eq!(callbacks.get(), 1);
    }
}
impl ITfCompositionSink_Impl for AcquisitionSink_Impl {
    fn OnCompositionTerminated(
        &self,
        ec: u32,
        composition: Ref<ITfComposition>,
    ) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        if !self.attached.get() {
            // The provider can terminate the composition before returning its
            // object. No unregistered object may become a live Draft afterward.
            self.terminated.set(true);
            return Ok(());
        }
        unsafe {
            self.delegate
                .OnCompositionTerminated(ec, composition.as_ref())
        }
    }
}

struct AcquisitionOwner {
    epoch: u64,
    context: ITfContext,
    cookie: u32,
}
impl AcquisitionOwner {
    fn retire_failed(&self, shared: &Shared) {
        let has_composition = {
            let state = shared.borrow();
            // Reentrant Deactivate may revoke the generation without removing
            // this candidate's resources. Conversely, a newer epoch must never
            // be touched by the old request's cleanup.
            if state.epoch != self.epoch
                || state.context.as_ref() != Some(&self.context)
                || state.text_cookie != Some(self.cookie)
            {
                return;
            }
            state.composition.is_some()
        };
        if has_composition {
            end_owned(shared);
        } else {
            release_composition(shared, self.epoch, None);
        }
    }
}

unsafe fn acquire_composition(
    edit: &EditBlock_Impl,
    track: &Track,
    snapshot: &BlockSnapshot,
    ec: u32,
    mutation: &mut Mutation,
    phase: &mut String,
    acquired_owner: &mut Option<AcquisitionOwner>,
) -> windows::core::Result<()> {
    unsafe {
        let shared = &edit.shared;
        let r = &edit.request;
        let context = &edit.context;
        let verified = track.verified_content(ec, phase)?;
        if verified.semantic_text != snapshot.text
            || verified.content_version != snapshot.version
            || verified.selection_start != snapshot.selection_start
            || verified.selection_end != snapshot.selection_end
        {
            *phase = "blockSnapshotChanged".into();
            return Err(failure(TF_E_LOCKED));
        }
        let original = range_position(context, &track.range, ec)?;
        if verified.native_end != original.end {
            return Err(failure(TF_E_DISCONNECTED));
        }
        // Paired reads may confirm identity, never justify moving endpoints.
        if track.verified_content(ec, phase)? != verified {
            return Err(failure(TF_E_LOCKED));
        }
        edit_authorized(shared, r, edit.connection, edit.generation)?;
        let native = range_text(&track.range, ec)?;
        if native != verified.native_text {
            *phase = "blockSnapshotChanged".into();
            return Err(failure(TF_E_LOCKED));
        }
        let text = CompositionText::from_verified(&verified.semantic_text, &native)
            .ok_or_else(|| failure(TF_E_LOCKED))?;
        let selected = selection(context, ec)?;
        let selected_position = range_position(context, &selected, ec)?;
        edit_authorized(shared, r, edit.connection, edit.generation)?;
        let epoch = COMPOSITION_EPOCH.fetch_add(1, Ordering::AcqRel) + 1;
        let text_sink: ITfTextEditSink = TextSink {
            state: shared.clone(),
            epoch,
            _lease: Lease::new(),
        }
        .into();
        let source: ITfSource = context.cast()?;
        let cookie = source.AdviseSink(&ITfTextEditSink::IID, &text_sink)?;
        if let Err(error) = edit_authorized(shared, r, edit.connection, edit.generation) {
            let _ = source.UnadviseSink(cookie);
            return Err(error);
        }
        {
            let mut state = shared.borrow_mut();
            if state.composition.is_some() || state.text_cookie.is_some() || state.version.owned {
                drop(state);
                let _ = source.UnadviseSink(cookie);
                return Err(failure(TF_E_DISCONNECTED));
            }
            state.epoch = epoch;
            *acquired_owner = Some(AcquisitionOwner {
                epoch,
                context: context.clone(),
                cookie,
            });
            state.context = Some(context.clone());
            state.text_cookie = Some(cookie);
            state.expected = native.clone();
            state.text = Some(text);
            state.selection = Some(selected);
            state.backup = None; // Reacquisition can never erase the user's baseline.
            state.prepared = None;
            state.empty_range = None;
            state.native_position = None;
            state.own_change = None;
            state.ownership_loss = None;
            state.ownership_detail = None;
            state.key_handoff = false;
            state.materialize_allowed = true;
            state.preserved_attributes.clear();
            state.version = DraftVersion {
                task: r.task.clone(),
                turn: r.turn,
                revision: r.revision,
                operation: r.operation,
                owned: true,
            };
        }
        *phase = "blockStartComposition".into();
        *mutation = Mutation::Uncertain;
        let delegate: ITfCompositionSink = CompositionSink {
            state: shared.clone(),
            epoch,
            _lease: Lease::new(),
        }
        .into();
        let attached = Rc::new(Cell::new(false));
        let terminated = Rc::new(Cell::new(false));
        let sink: ITfCompositionSink = AcquisitionSink {
            delegate,
            attached: attached.clone(),
            terminated: terminated.clone(),
            _lease: Lease::new(),
        }
        .into();
        let owner: ITfContextComposition = context.cast()?;
        #[cfg(feature = "acceptance")]
        block_faults::acquisition_without_object(shared)?;
        // The Windows wrapper rejects a successful HRESULT with a null object.
        let composition = owner.StartComposition(ec, &track.range, &sink)?;
        // StartComposition may synchronously terminate the candidate or reenter
        // the service. Never attach its returned object to a newer owner.
        let still_owned = {
            let state = shared.borrow();
            !terminated.get()
                && state.epoch == epoch
                && state.generation == edit.generation
                && state.version.owned
                && state.version.operation == r.operation
                && state.version.task == r.task
                && state.composition.is_none()
        };
        if !still_owned {
            // This object is ours, but it cannot authorize a write. Retain it
            // for the existing retirement path if ending it fails.
            if composition.EndComposition(ec).is_err() {
                let mut state = shared.borrow_mut();
                if state.epoch == epoch && state.composition.is_none() {
                    state.composition = Some(composition);
                    // It now belongs to the existing retirement owner. A later
                    // real termination must release that owner's resources even
                    // though this candidate never became writable.
                    attached.set(true);
                }
            }
            return Err(failure(TF_E_DISCONNECTED));
        }
        shared.borrow_mut().composition = Some(composition.clone());
        attached.set(true);
        edit_authorized(shared, r, edit.connection, edit.generation)?;
        let actual = composition.GetRange()?;
        if actual.CompareStart(ec, &track.range, TF_ANCHOR_START)? != 0
            || actual.CompareEnd(ec, &track.range, TF_ANCHOR_END)? != 0
            || range_text(&actual, ec)? != native
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let after_selection = range_position(context, &selection(context, ec)?, ec)?;
        if after_selection.start != selected_position.start
            || after_selection.end != selected_position.end
            || track.data.borrow().lease != snapshot.lease
        {
            *phase = "blockSnapshotChanged".into();
            *mutation = Mutation::NotAttempted;
            return Err(failure(TF_E_LOCKED));
        }
        edit_authorized(shared, r, edit.connection, edit.generation)?;
        let mut data = track.data.borrow_mut();
        if !data.valid || data.version != snapshot.version {
            return Err(failure(TF_E_DISCONNECTED));
        }
        data.text = native;
        data.lease += 1;
        data.writable = true;
        data.reacquired = true;
        *mutation = Mutation::NotAttempted;
        Ok(())
    }
}

pub(super) unsafe fn selection_for_write(
    shared: &Shared,
    context: &ITfContext,
    range: &ITfRange,
    ec: u32,
    next: &str,
) -> windows::core::Result<Option<u32>> {
    unsafe {
        let (track, before) = {
            let state = shared.borrow();
            let Some(track) = state.block.clone() else {
                return Ok(None);
            };
            if !track.data.borrow().reacquired {
                return Ok(None);
            }
            (
                track,
                state
                    .text
                    .clone()
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?,
            )
        };
        if !track.data.borrow().valid {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let native = range_text(range, ec)?;
        let (_, end) = relative_selection(context, range, ec)?;
        let at = before
            .position(&native, end, false)
            .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
        map_endpoint(&before.semantic, next, at)
            .map(Some)
            .ok_or_else(|| failure(TF_E_DISCONNECTED))
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
        let mut phase = "block".into();
        let mut acquired_owner = None;
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
                // Binding records an already-owned write, not an external
                // editor baseline. Its provenance is the active composition
                // and this writer's exact text/position map; UIA may still be
                // publishing the previous renderer snapshot here.
                let (composition, written) = {
                    let state = shared.borrow();
                    (
                        state
                            .composition
                            .clone()
                            .ok_or_else(|| failure(TF_E_DISCONNECTED))?,
                        state
                            .text
                            .clone()
                            .ok_or_else(|| failure(TF_E_DISCONNECTED))?,
                    )
                };
                let owned = composition.GetRange()?;
                if owned.CompareStart(ec, &track.range, TF_ANCHOR_START)? != 0
                    || owned.CompareEnd(ec, &track.range, TF_ANCHOR_END)? != 0
                {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                let actual = range_text(&owned, ec)?;
                let (start, end) = relative_selection(&self.context, &owned, ec)?;
                let start = written
                    .position(&actual, start, false)
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
                let end = written
                    .position(&actual, end, false)
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
                let snapshot = {
                    let data = track.data.borrow();
                    if !data.valid || data.text != actual {
                        return Err(failure(TF_E_DISCONNECTED));
                    }
                    BlockSnapshot {
                        id: data.id.clone(),
                        epoch: data.epoch,
                        version: data.version,
                        lease: data.lease,
                        text: written.semantic,
                        selection_start: start,
                        selection_end: end,
                        writable: false,
                    }
                };
                edit_authorized(shared, r, self.connection, self.generation)?;
                let mut state = shared.borrow_mut();
                if state.epoch != epoch
                    || !state.version.owned
                    || state.version.task != r.task
                    || state.version.turn != r.turn
                    || state.block.is_some()
                    || state.expected != track.data.borrow().text
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
                phase = "blockSnapshotChanged".into();
                if snapshot.version != command.version
                    || snapshot.text != command.expected
                    || snapshot.selection_start != command.selection_start
                    || snapshot.selection_end != command.selection_end
                    || (command.action == BlockAction::Acquire
                        && (shared.borrow().version.owned || shared.borrow().composition.is_some()))
                {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                let owner: ITfContextComposition = self.context.cast()?;
                let mut compositions = [None];
                let mut fetched = 0;
                owner
                    .EnumCompositions()?
                    .Next(&mut compositions, &mut fetched)?;
                if command.action == BlockAction::Acquire && fetched != 0 {
                    phase = "blockInputMethodBusy".into();
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
                    acquire_composition(
                        self,
                        &track,
                        &snapshot,
                        ec,
                        &mut mutation,
                        &mut phase,
                        &mut acquired_owner,
                    )?;
                    // Do not re-read UIA inside compositionstart. The exact native
                    // readback above confirms this acquisition; later writes
                    // still require the normal live Draft lease.
                    let data = track.data.borrow();
                    return Ok(BlockSnapshot {
                        lease: data.lease,
                        writable: true,
                        ..snapshot
                    });
                }
                BlockAction::Apply => {
                    if !track.data.borrow().writable
                        || snapshot.lease != command.lease
                        || r.text.is_empty()
                    {
                        return Err(failure(TF_E_DISCONNECTED));
                    }
                    let composition = shared
                        .borrow()
                        .composition
                        .clone()
                        .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
                    let range = composition.GetRange()?;
                    let epoch = shared.borrow().epoch;
                    let mut update = r.clone();
                    update.action = Action::Update;
                    update.block = None;
                    let mut writer_phase = "blockUpdate";
                    write_composing(
                        shared,
                        &self.context,
                        &range,
                        &update,
                        self.generation,
                        epoch,
                        ec,
                        &mut mutation,
                        &mut writer_phase,
                        &r.text,
                        true,
                    )
                    .inspect_err(|_| phase = writer_phase.into())?;
                    let data = track.data.borrow();
                    return Ok(BlockSnapshot {
                        text: r.text.clone(),
                        version: data.version,
                        lease: data.lease,
                        writable: false,
                        selection_start: r.text.encode_utf16().count() as u32,
                        selection_end: r.text.encode_utf16().count() as u32,
                        ..snapshot
                    });
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
            Err(error) => {
                let track = self.shared.borrow().block.clone();
                let detail = if let Some(track) = track {
                    unsafe {
                        evidence::capture(
                            &self.context,
                            &track.range,
                            &track.field,
                            &track.data,
                            ec,
                            ("request", &phase, Some(error.code())),
                        );
                    }
                    evidence::suffix(&track.data)
                } else {
                    String::new()
                };
                complete(
                    &self.shared,
                    &self.request,
                    self.connection,
                    false,
                    &format!("{phase}:{:08x}{detail}", error.code().0 as u32),
                    mutation,
                    None,
                );
                if let Some(owner) = acquired_owner {
                    owner.retire_failed(&self.shared);
                }
            }
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
