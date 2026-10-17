//! Exercises only the owned fixture window, without machine registration.
use super::*;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};

fn request(
    pipe: &Pipe,
    capability: &str,
    task: &str,
    operation: u64,
    block: BlockCommand,
    text: &str,
) -> Result<Reply, Box<dyn std::error::Error>> {
    let r = Request {
        revoke: None,
        block: Some(block),
        engine: fixture_binding(),
        protocol: PROTOCOL,
        capability: capability.into(),
        task: task.into(),
        turn: 1,
        operation,
        revision: operation,
        deadline_tick: unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
            + 2000,
        action: Action::Inspect,
        text: text.into(),
        prefix: None,
    };
    pipe.send(&serde_json::to_vec(&r)?)?;
    let started = Instant::now();
    loop {
        pump(Duration::from_millis(10));
        if let Some(data) = pipe.receive()? {
            let result: Reply = serde_json::from_slice(&data)?;
            if result.operation != operation {
                return Err("block operation mismatch".into());
            }
            return Ok(result);
        }
        if started.elapsed() > Duration::from_secs(2) {
            return Err("block fixture timeout".into());
        }
    }
}
fn command(snapshot: &BlockSnapshot, action: BlockAction) -> BlockCommand {
    BlockCommand {
        lease: snapshot.lease,
        id: snapshot.id.clone(),
        action,
        epoch: snapshot.epoch,
        version: snapshot.version,
        expected: snapshot.text.clone(),
        selection_start: snapshot.selection_start,
        selection_end: snapshot.selection_end,
    }
}
fn snapshot(reply: Reply) -> Result<BlockSnapshot, Box<dyn std::error::Error>> {
    if !reply.applied {
        return Err(format!("block fixture rejected: {}", reply.code).into());
    }
    reply.block.ok_or_else(|| "missing block snapshot".into())
}
pub(super) fn run(
    editor: HWND,
    manager: &ITfThreadMgr,
    pipe: &Pipe,
    capability: &str,
    module: windows::Win32::Foundation::HMODULE,
) -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("SAYWEAVE_BLOCK_FIXTURE").as_deref() == Ok("native-scope") {
        return native_scope_cases(editor, manager, pipe, capability, module);
    }
    if std::env::var("SAYWEAVE_BLOCK_FIXTURE").as_deref() == Ok("unified") {
        return unified(editor, pipe, capability, module);
    }
    let task = uuid::Uuid::new_v4().to_string();
    let id = uuid::Uuid::new_v4().to_string();
    for (operation, revision, action, text) in
        [(1, 0, Action::Begin, ""), (2, 1, Action::Update, "周三")]
    {
        let result = exchange(
            pipe, capability, &task, 1, operation, revision, action, text, false,
        )?;
        if !result.applied {
            return Err(format!("block setup: {}", result.code).into());
        }
    }
    let initial = snapshot(request(
        pipe,
        capability,
        &task,
        3,
        BlockCommand {
            lease: 0,
            id,
            action: BlockAction::Bind,
            epoch: 0,
            version: 0,
            expected: String::new(),
            selection_start: 0,
            selection_end: 0,
        },
        "",
    )?)?;
    println!(
        "block bind body={} version={}",
        initial.text == "周三",
        initial.version
    );
    let fault: unsafe extern "system" fn(u32) = unsafe {
        std::mem::transmute(
            windows::Win32::System::LibraryLoader::GetProcAddress(
                module,
                windows::core::s!("SayweaveBlockFixtureFault"),
            )
            .ok_or("missing range fixture hook")?,
        )
    };
    unsafe {
        fault(6);
    }
    let streamed = exchange(
        pipe,
        capability,
        &task,
        1,
        4,
        2,
        Action::Update,
        "周三开会",
        false,
    )?;
    if !streamed.applied {
        return Err(format!("bound block streaming update: {}", streamed.code).into());
    }
    let handoff = exchange(pipe, capability, &task, 1, 5, 2, Action::Handoff, "", false)?;
    if !handoff.applied {
        return Err(format!("block handoff: {}", handoff.code).into());
    }
    let handed_reply = request(
        pipe,
        capability,
        &task,
        6,
        command(&initial, BlockAction::Read),
        "",
    )?;
    if !handed_reply.applied && std::env::args().nth(3).as_deref() == Some("plain") {
        if handed_reply.mutation != Mutation::NotAttempted || editor_text(editor)? != "周三开会"
        {
            return Err("unsupported legacy anchor changed body".into());
        }
        let released = request(
            pipe,
            capability,
            &task,
            7,
            command(&initial, BlockAction::Release),
            "",
        )?;
        if !released.applied {
            return Err("unsupported legacy anchor cleanup failed".into());
        }
        println!(
            "block legacy EDIT unsupported anchor safely refused; retained body/released track=true"
        );
        return Ok(());
    }
    let handed = snapshot(handed_reply)?;
    if handed.text != "周三开会" || editor_text(editor)? != "周三开会" {
        return Err("handoff changed body".into());
    }
    println!("block handoff anchors retained=true");
    unsafe {
        SendMessageW(editor, 0xB1, Some(WPARAM(1)), Some(LPARAM(2)));
        SendMessageW(
            editor,
            0xC2,
            Some(WPARAM(1)),
            Some(LPARAM(windows::core::w!("四").as_ptr() as isize)),
        );
    }
    pump(Duration::from_millis(50));
    let edited = snapshot(request(
        pipe,
        capability,
        &task,
        7,
        command(&handed, BlockAction::Read),
        "",
    )?)?;
    if edited.text != "周四开会" || edited.version <= handed.version {
        return Err("user edit not tracked".into());
    }
    let stale = request(
        pipe,
        capability,
        &task,
        8,
        command(&handed, BlockAction::Acquire),
        "",
    )?;
    if stale.applied || stale.mutation != Mutation::NotAttempted {
        return Err("stale acquire accepted".into());
    }
    // Place the caret at a proven block boundary before the replacement.
    unsafe {
        SendMessageW(editor, 0xB1, Some(WPARAM(4)), Some(LPARAM(4)));
    }
    pump(Duration::from_millis(30));
    let current = snapshot(request(
        pipe,
        capability,
        &task,
        9,
        command(&edited, BlockAction::Read),
        "",
    )?)?;
    let lease = snapshot(request(
        pipe,
        capability,
        &task,
        10,
        command(&current, BlockAction::Acquire),
        "",
    )?)?;
    let repeated = request(
        pipe,
        capability,
        &task,
        11,
        command(&current, BlockAction::Acquire),
        "",
    )?;
    if repeated.applied || repeated.mutation != Mutation::NotAttempted {
        return Err("already owned composition acquired a second owner".into());
    }
    let mut obsolete_lease = command(&lease, BlockAction::Apply);
    obsolete_lease.lease = lease.lease.saturating_sub(1);
    let obsolete = request(pipe, capability, &task, 12, obsolete_lease, "过期正文")?;
    if obsolete.applied
        || obsolete.mutation != Mutation::NotAttempted
        || editor_text(editor)? != "周四开会"
    {
        return Err("previous lease wrote after reacquire".into());
    }
    let applied = snapshot(request(
        pipe,
        capability,
        &task,
        13,
        command(&lease, BlockAction::Apply),
        "周四下午开会",
    )?)?;
    if editor_text(editor)? != "周四下午开会" || applied.text != "周四下午开会" {
        return Err("actual block replacement mismatch".into());
    }
    let applied = timing_cases(editor, pipe, capability, &task, applied, module)?;
    let acquired = snapshot(request(
        pipe,
        capability,
        &task,
        27,
        command(&applied, BlockAction::Read),
        "",
    )?)?;
    let multiline = "周四下午两点开会\n下一行 👩‍💻 e\u{301}";
    let updated = exchange(
        pipe,
        capability,
        &task,
        1,
        28,
        28,
        Action::Update,
        multiline,
        false,
    )?;
    if !updated.applied {
        return Err(format!("multiline shared update: {}", updated.code).into());
    }
    if !exchange(
        pipe,
        capability,
        &task,
        1,
        29,
        29,
        Action::Handoff,
        "",
        false,
    )?
    .applied
    {
        return Err("multiline shared composition did not retire".into());
    }
    pump(Duration::from_millis(50));
    let applied = snapshot(request(
        pipe,
        capability,
        &task,
        30,
        command(&acquired, BlockAction::Read),
        "",
    )?)?;
    let normalize = |text: &str| text.replace("\r\n", "\n").replace('\r', "\n");
    if normalize(&editor_text(editor)?) != multiline
        || normalize(&applied.text) != multiline
        || applied.selection_start != applied.text.encode_utf16().count() as u32
        || applied.selection_end != applied.selection_start
    {
        return Err("multiline Unicode body/caret mismatch".into());
    }
    println!("block multiline/UTF16/grapheme-caret=true");
    unsafe {
        let original = manager.GetFocus()?;
        let other = manager.CreateDocumentMgr()?;
        manager.SetFocus(&other)?;
        manager.SetFocus(&original)?;
    }
    let returned = request(
        pipe,
        capability,
        &task,
        31,
        command(&applied, BlockAction::Acquire),
        "",
    )?;
    if returned.applied
        || returned.mutation != Mutation::NotAttempted
        || normalize(&editor_text(editor)?) != multiline
    {
        return Err("leaving and returning revived block".into());
    }
    let release = request(
        pipe,
        capability,
        &task,
        32,
        command(&applied, BlockAction::Release),
        "",
    )?;
    if !release.applied {
        return Err("block release failed".into());
    }
    println!(
        "block manual-edit/read/stale-refusal/lease-rotation/reacquire/actual-write/focus-retirement/release=true"
    );
    boundary_cases(editor, pipe, capability, module)?;
    Ok(())
}

fn native_scope_cases(
    editor: HWND,
    manager: &ITfThreadMgr,
    pipe: &Pipe,
    capability: &str,
    module: windows::Win32::Foundation::HMODULE,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows::{
        Win32::UI::{Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
        core::w,
    };
    let fault: unsafe extern "system" fn(u32) = unsafe {
        std::mem::transmute(
            windows::Win32::System::LibraryLoader::GetProcAddress(
                module,
                windows::core::s!("SayweaveBlockFixtureFault"),
            )
            .ok_or("missing native scope fixture hook")?,
        )
    };
    unsafe {
        fault(11);
    }
    let mut operation = 0;
    let task = uuid::Uuid::new_v4().to_string();
    let mut send = |action: Action, block: Option<BlockCommand>, text: &str| {
        operation += 1;
        let request = frame(capability, &task, operation, block, action, text, None);
        pipe.send(&serde_json::to_vec(&request)?)?;
        let started = Instant::now();
        loop {
            pump(Duration::from_millis(10));
            if let Some(data) = pipe.receive()? {
                let reply: Reply = serde_json::from_slice(&data)?;
                if reply.operation != operation {
                    return Err("native scope operation mismatch".into());
                }
                return Ok::<Reply, Box<dyn std::error::Error>>(reply);
            }
            if started.elapsed() >= Duration::from_secs(2) {
                return Err("native scope fixture timeout".into());
            }
        }
    };
    unsafe {
        SendMessageW(
            editor,
            WM_SETTEXT,
            Some(WPARAM(0)),
            Some(LPARAM(w!("左｜｜右").as_ptr() as isize)),
        );
        SendMessageW(editor, 0xB1, Some(WPARAM(2)), Some(LPARAM(2)));
    }
    pump(Duration::from_millis(40));
    if !send(Action::Begin, None, "")?.applied {
        return Err("native scope begin failed".into());
    }
    let bind = BlockCommand {
        id: uuid::Uuid::new_v4().to_string(),
        action: BlockAction::Bind,
        epoch: 0,
        version: 0,
        lease: 0,
        expected: String::new(),
        selection_start: 0,
        selection_end: 0,
    };
    let mut current = snapshot(send(Action::Update, Some(bind), "周三开会")?)?;
    if editor_text(editor)? != "左｜周三开会｜右" {
        return Err("native scope first body differs".into());
    }
    println!(
        "native-scope first actual={:?} provider=unavailable",
        editor_text(editor)?
    );
    let edits = [
        (1, 2, "五", "周五开会", "下周一上午开会"),
        (3, 5, "", "下周一开会", "下周一下午三点开会"),
        (5, 6, "两", "下周一下午两点开会", "下周一线上四点开会"),
        (2, 3, "二", "下周二线上四点开会", "下周二线上五点开会"),
        (0, 9, "明天开会", "明天开会", "明天下午开会"),
    ];
    for (start, end, replacement, edited, target) in edits {
        if !send(Action::Handoff, None, "")?.applied {
            return Err("native scope handoff failed".into());
        }
        pump(Duration::from_millis(40));
        let replacement: Vec<u16> = replacement.encode_utf16().chain(Some(0)).collect();
        unsafe {
            SendMessageW(
                editor,
                0xB1,
                Some(WPARAM(start + 2)),
                Some(LPARAM((end + 2) as isize)),
            );
            SendMessageW(
                editor,
                0xC2,
                Some(WPARAM(1)),
                Some(LPARAM(replacement.as_ptr() as isize)),
            );
        }
        pump(Duration::from_millis(40));
        if editor_text(editor)? != format!("左｜{edited}｜右") {
            return Err("native scope edit body differs".into());
        }
        unsafe {
            let end = 2 + edited.encode_utf16().count();
            SendMessageW(editor, 0xB1, Some(WPARAM(end)), Some(LPARAM(end as isize)));
        }
        pump(Duration::from_millis(30));
        let read = snapshot(send(
            Action::Inspect,
            Some(command(&current, BlockAction::Read)),
            "",
        )?)?;
        if read.text != edited {
            return Err(format!("native scope edited snapshot differs: {:?}", read.text).into());
        }
        let acquired = snapshot(send(
            Action::Inspect,
            Some(command(&read, BlockAction::Acquire)),
            "",
        )?)?;
        current = snapshot(send(
            Action::Inspect,
            Some(command(&acquired, BlockAction::Apply)),
            target,
        )?)?;
        pump(Duration::from_millis(40));
        if editor_text(editor)? != format!("左｜{target}｜右") || current.text != target {
            return Err("native scope reacquisition body differs".into());
        }
        println!(
            "native-scope reacquired actual={:?} provider=unavailable",
            editor_text(editor)?
        );
    }
    if !send(Action::Handoff, None, "")?.applied {
        return Err("native scope final handoff failed".into());
    }
    pump(Duration::from_millis(30));
    let original_body = editor_text(editor)?;
    let other_body: Vec<u16> = original_body.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let other = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("RICHEDIT50W"),
            windows::core::PCWSTR(other_body.as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_MULTILINE as u32),
            10,
            185,
            480,
            35,
            Some(GetAncestor(editor, GA_ROOT)),
            None,
            None,
            None,
        )?;
        SendMessageW(
            other,
            WM_USER + 204,
            Some(WPARAM(0x10000)),
            Some(LPARAM(0x10000)),
        );
        let _ = SetFocus(Some(other));
        pump(Duration::from_millis(40));
        let _ = SetFocus(Some(editor));
        pump(Duration::from_millis(40));
        DestroyWindow(other)?;
    }
    let denied = send(
        Action::Inspect,
        Some(command(&current, BlockAction::Acquire)),
        "",
    )?;
    if denied.applied
        || denied.mutation != Mutation::NotAttempted
        || editor_text(editor)? != original_body
    {
        return Err("native scope field switch revived old block".into());
    }
    if !send(
        Action::Inspect,
        Some(command(&current, BlockAction::Release)),
        "",
    )?
    .applied
    {
        return Err("native scope release failed".into());
    }
    println!("native-scope field-switch/return refused actual={original_body:?}");
    // A text reset alone is an observation, not a simulated TSF document
    // teardown. Pop/Push lifetime events are tested on a document owned by
    // the TSF test, not by removing RichEdit's private primary context.
    for outside in [true, false] {
        native_scope_reset_case(editor, manager, pipe, capability, outside)?;
    }
    Ok(())
}

fn native_scope_reset_case(
    editor: HWND,
    manager: &ITfThreadMgr,
    pipe: &Pipe,
    capability: &str,
    outside: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows::Win32::UI::WindowsAndMessaging::*;
    let task = uuid::Uuid::new_v4().to_string();
    let mut operation = 0;
    let mut send = |action: Action, block: Option<BlockCommand>, text: &str| {
        operation += 1;
        pipe.send(&serde_json::to_vec(&frame(
            capability, &task, operation, block, action, text, None,
        ))?)?;
        let started = Instant::now();
        loop {
            pump(Duration::from_millis(10));
            if let Some(data) = pipe.receive()? {
                let reply: Reply = serde_json::from_slice(&data)?;
                if reply.operation != operation {
                    return Err("scope reset operation mismatch".into());
                }
                return Ok::<Reply, Box<dyn std::error::Error>>(reply);
            }
            if started.elapsed() >= Duration::from_secs(2) {
                return Err("scope reset timeout".into());
            }
        }
    };
    let initial = if outside { "左｜｜右" } else { "" };
    let body = if outside {
        "左｜周三开会｜右"
    } else {
        "周三开会"
    };
    let initial: Vec<u16> = initial.encode_utf16().chain(Some(0)).collect();
    unsafe {
        SendMessageW(
            editor,
            WM_SETTEXT,
            Some(WPARAM(0)),
            Some(LPARAM(initial.as_ptr() as isize)),
        );
        let at = if outside { 2 } else { 0 };
        SendMessageW(editor, 0xB1, Some(WPARAM(at)), Some(LPARAM(at as isize)));
    }
    pump(Duration::from_millis(30));
    if !send(Action::Begin, None, "")?.applied {
        return Err("scope reset begin failed".into());
    }
    let bind = BlockCommand {
        id: uuid::Uuid::new_v4().to_string(),
        action: BlockAction::Bind,
        epoch: 0,
        version: 0,
        lease: 0,
        expected: String::new(),
        selection_start: 0,
        selection_end: 0,
    };
    let current = snapshot(send(Action::Update, Some(bind), "周三开会")?)?;
    if !send(Action::Handoff, None, "")?.applied {
        return Err("scope reset handoff failed".into());
    }
    pump(Duration::from_millis(30));
    let document = unsafe { manager.GetFocus()? };
    let before = unsafe { document.GetTop()? };
    let before_read = send(
        Action::Inspect,
        Some(command(&current, BlockAction::Read)),
        "",
    )?;
    if !before_read.applied {
        return Err(format!(
            "native scope intact whole-field read failed outside={outside}: {}",
            before_read.code
        )
        .into());
    }
    let replacement: Vec<u16> = body.encode_utf16().chain(Some(0)).collect();
    unsafe {
        SendMessageW(
            editor,
            WM_SETTEXT,
            Some(WPARAM(0)),
            Some(LPARAM(replacement.as_ptr() as isize)),
        );
    }
    pump(Duration::from_millis(40));
    let same_context = unsafe { manager.GetFocus()?.GetTop()? == before };
    let read = send(
        Action::Inspect,
        Some(command(&current, BlockAction::Read)),
        "",
    )?;
    let acquire = send(
        Action::Inspect,
        Some(command(&current, BlockAction::Acquire)),
        "",
    )?;
    println!(
        "native-scope text-reset outside={outside} same_context={same_context} read={} read_code={} acquire={} actual={:?}",
        read.applied,
        read.code,
        acquire.applied,
        editor_text(editor)?
    );
    if editor_text(editor)? != body {
        return Err("native scope reset changed the host body".into());
    }
    if acquire.applied && !send(Action::Handoff, None, "")?.applied {
        return Err("text-reset observation failed to hand off acquired composition".into());
    }
    if !send(
        Action::Inspect,
        Some(command(&current, BlockAction::Release)),
        "",
    )?
    .applied
    {
        return Err("scope reset release failed".into());
    }
    Ok(())
}
fn boundary_cases(
    editor: HWND,
    pipe: &Pipe,
    capability: &str,
    module: windows::Win32::Foundation::HMODULE,
) -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        SendMessageW(
            editor,
            0xC,
            Some(WPARAM(0)),
            Some(LPARAM(windows::core::w!("左右").as_ptr() as isize)),
        );
        SendMessageW(editor, 0xB1, Some(WPARAM(1)), Some(LPARAM(1)));
    }
    pump(Duration::from_millis(40));
    let task = uuid::Uuid::new_v4().to_string();
    for (op, rev, action, text) in [
        (1, 0, Action::Begin, ""),
        (2, 1, Action::Update, "周三开会"),
    ] {
        let reply = exchange(pipe, capability, &task, 1, op, rev, action, text, false)?;
        if !reply.applied {
            return Err(format!("boundary setup: {}", reply.code).into());
        }
    }
    let initial = snapshot(request(
        pipe,
        capability,
        &task,
        3,
        BlockCommand {
            id: uuid::Uuid::new_v4().to_string(),
            action: BlockAction::Bind,
            epoch: 0,
            version: 0,
            lease: 0,
            expected: String::new(),
            selection_start: 0,
            selection_end: 0,
        },
        "",
    )?)?;
    let handoff = exchange(pipe, capability, &task, 1, 4, 1, Action::Handoff, "", false)?;
    if !handoff.applied {
        return Err("boundary handoff failed".into());
    }
    let handed = snapshot(request(
        pipe,
        capability,
        &task,
        5,
        command(&initial, BlockAction::Read),
        "",
    )?)?;
    let fault: unsafe extern "system" fn(u32) = unsafe {
        std::mem::transmute(
            windows::Win32::System::LibraryLoader::GetProcAddress(
                module,
                windows::core::s!("SayweaveBlockFixtureFault"),
            )
            .ok_or("missing coarse fixture hook")?,
        )
    };
    unsafe {
        SendMessageW(editor, 0xB1, Some(WPARAM(2)), Some(LPARAM(3)));
    }
    pump(Duration::from_millis(20));
    unsafe {
        fault(5);
        SendMessageW(
            editor,
            0xC2,
            Some(WPARAM(1)),
            Some(LPARAM(windows::core::w!("五").as_ptr() as isize)),
        );
    }
    pump(Duration::from_millis(30));
    unsafe {
        SendMessageW(editor, 0xB1, Some(WPARAM(5)), Some(LPARAM(5)));
    }
    pump(Duration::from_millis(50));
    let edited = snapshot(request(
        pipe,
        capability,
        &task,
        6,
        command(&handed, BlockAction::Read),
        "",
    )?)?;
    if edited.text != "周五开会" || editor_text(editor)? != "左周五开会右" {
        return Err("coarse notification lost scoped user edit".into());
    }
    let acquired = snapshot(request(
        pipe,
        capability,
        &task,
        7,
        command(&edited, BlockAction::Acquire),
        "",
    )?)?;
    let applied = snapshot(request(
        pipe,
        capability,
        &task,
        8,
        command(&acquired, BlockAction::Apply),
        "下周一上午开会",
    )?)?;
    if editor_text(editor)? != "左下周一上午开会右" || applied.text != "下周一上午开会"
    {
        return Err("coarse notification replacement changed outside text".into());
    }
    if !exchange(pipe, capability, &task, 1, 9, 9, Action::Handoff, "", false)?.applied {
        return Err("boundary composition handoff failed".into());
    }
    pump(Duration::from_millis(30));
    unsafe {
        SendMessageW(editor, 0xB1, Some(WPARAM(0)), Some(LPARAM(1)));
        SendMessageW(
            editor,
            0xC2,
            Some(WPARAM(1)),
            Some(LPARAM(windows::core::w!("改").as_ptr() as isize)),
        );
    }
    pump(Duration::from_millis(50));
    let denied = request(
        pipe,
        capability,
        &task,
        10,
        command(&applied, BlockAction::Read),
        "",
    )?;
    let write = request(
        pipe,
        capability,
        &task,
        11,
        command(&applied, BlockAction::Apply),
        "不能写入",
    )?;
    if denied.applied
        || write.applied
        || write.mutation != Mutation::NotAttempted
        || editor_text(editor)? != "改下周一上午开会右"
    {
        return Err(format!(
            "outside edit refusal differs: read={} write={} mutation={:?} body={:?}",
            denied.applied,
            write.applied,
            write.mutation,
            editor_text(editor)?
        )
        .into());
    }
    if !request(
        pipe,
        capability,
        &task,
        12,
        command(&applied, BlockAction::Release),
        "",
    )?
    .applied
    {
        return Err("boundary track release failed".into());
    }
    println!("block coarse-notice/edit/read/reacquire/body/outside-refusal=true");
    Ok(())
}
fn frame(
    capability: &str,
    task: &str,
    operation: u64,
    block: Option<BlockCommand>,
    action: Action,
    text: &str,
    revoke: Option<Revoke>,
) -> Request {
    let revision = revoke.as_ref().map_or(operation, |target| target.revision);
    Request {
        protocol: PROTOCOL,
        engine: fixture_binding(),
        capability: capability.into(),
        task: task.into(),
        turn: 1,
        operation,
        revision,
        deadline_tick: unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
            + 2000,
        block,
        action,
        text: text.into(),
        prefix: None,
        revoke,
    }
}
fn pair(pipe: &Pipe, a: u64, b: u64) -> Result<(Reply, Reply), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let mut first = None;
    let mut second = None;
    while first.is_none() || second.is_none() {
        pump(Duration::from_millis(10));
        if let Some(data) = pipe.receive()? {
            let r: Reply = serde_json::from_slice(&data)?;
            if r.operation == a {
                first = Some(r);
            } else if r.operation == b {
                second = Some(r);
            } else {
                return Err("unexpected timing receipt".into());
            }
        }
        if started.elapsed() > Duration::from_secs(2) {
            return Err("timing receipts timeout".into());
        }
    }
    Ok((first.unwrap(), second.unwrap()))
}
fn timing_cases(
    editor: HWND,
    pipe: &Pipe,
    capability: &str,
    task: &str,
    mut current: BlockSnapshot,
    module: windows::Win32::Foundation::HMODULE,
) -> Result<BlockSnapshot, Box<dyn std::error::Error>> {
    use windows::Win32::System::LibraryLoader::GetProcAddress;
    let fault: unsafe extern "system" fn(u32) = unsafe {
        std::mem::transmute(
            GetProcAddress(module, windows::core::s!("SayweaveBlockFixtureFault"))
                .ok_or("missing timing hook")?,
        )
    };
    let flush: unsafe extern "system" fn() -> bool = unsafe {
        std::mem::transmute(
            GetProcAddress(module, windows::core::s!("SayweaveBlockFixtureFlush"))
                .ok_or("missing timing flush")?,
        )
    };
    for (kind, op) in [(3, 14), (4, 16)] {
        unsafe {
            fault(kind);
        }
        let denied = request(
            pipe,
            capability,
            task,
            op,
            command(&current, BlockAction::Read),
            "",
        )?;
        if denied.applied || denied.mutation != Mutation::NotAttempted {
            return Err("edit scheduling fault reported success".into());
        }
        current = snapshot(request(
            pipe,
            capability,
            task,
            op + 1,
            command(&current, BlockAction::Read),
            "",
        )?)?;
    }
    unsafe {
        fault(1);
    }
    pipe.send(&serde_json::to_vec(&frame(
        capability,
        task,
        18,
        Some(command(&current, BlockAction::Apply)),
        Action::Inspect,
        "不应显示的尾部",
        None,
    ))?)?;
    pump(Duration::from_millis(100));
    if pipe.receive()?.is_some() {
        return Err("deferred write already completed".into());
    }
    pipe.send(&serde_json::to_vec(&frame(
        capability,
        task,
        19,
        None,
        Action::Revoke,
        "",
        Some(Revoke {
            operation: 18,
            revision: 18,
        }),
    ))?)?;
    let (original, revoke) = pair(pipe, 18, 19)?;
    if original.applied || original.mutation != Mutation::NotAttempted || !revoke.applied {
        return Err("queued write not precisely revoked".into());
    }
    if !unsafe { flush() } {
        return Err("deferred callback flush failed".into());
    }
    pump(Duration::from_millis(80));
    if pipe.receive()?.is_some() || editor_text(editor)? != "周四下午开会" {
        return Err("revoked late callback changed body or duplicated receipt".into());
    }
    current = snapshot(request(
        pipe,
        capability,
        task,
        20,
        command(&current, BlockAction::Read),
        "",
    )?)?;
    if !exchange(
        pipe,
        capability,
        task,
        1,
        21,
        21,
        Action::Handoff,
        "",
        false,
    )?
    .applied
    {
        return Err("timing composition handoff failed".into());
    }
    current = snapshot(request(
        pipe,
        capability,
        task,
        22,
        command(&current, BlockAction::Read),
        "",
    )?)?;
    current = snapshot(request(
        pipe,
        capability,
        task,
        23,
        command(&current, BlockAction::Acquire),
        "",
    )?)?;
    unsafe {
        fault(2);
    }
    pipe.send(&serde_json::to_vec(&frame(
        capability,
        task,
        24,
        Some(command(&current, BlockAction::Apply)),
        Action::Inspect,
        "周四下午两点开会",
        None,
    ))?)?;
    let (original, revoke) = pair(pipe, 24, 25)?;
    if !original.applied
        || original.mutation != Mutation::Changed
        || !revoke.applied
        || revoke.code != "operationStarted"
    {
        return Err("started write lost its actual outcome".into());
    }
    current = snapshot(request(
        pipe,
        capability,
        task,
        26,
        command(&current, BlockAction::Read),
        "",
    )?)?;
    if editor_text(editor)? != "周四下午两点开会" {
        return Err("started write body mismatch".into());
    }
    println!(
        "block scheduling-failure/retry/queued-stop/late-callback/started-effect-receipt=true"
    );
    Ok(current)
}

/// Actual native input on the existing owned control, using ordinary Update
/// after Acquire. Checks are repeated after pumping host notifications.
fn unified(
    editor: HWND,
    pipe: &Pipe,
    capability: &str,
    module: windows::Win32::Foundation::HMODULE,
) -> Result<(), Box<dyn std::error::Error>> {
    let fault: unsafe extern "system" fn(u32) = unsafe {
        std::mem::transmute(
            windows::Win32::System::LibraryLoader::GetProcAddress(
                module,
                windows::core::s!("SayweaveBlockFixtureFault"),
            )
            .ok_or("missing acquisition fixture hook")?,
        )
    };
    // These sentinels are outside the owned composition in every round.
    unsafe {
        SendMessageW(
            editor,
            WM_SETTEXT,
            None,
            Some(LPARAM(windows::core::w!("左右").as_ptr() as isize)),
        );
        SendMessageW(editor, 0xB1, Some(WPARAM(1)), Some(LPARAM(1)));
    }
    pump(Duration::from_millis(30));
    let task = uuid::Uuid::new_v4().to_string();
    let mut op = 0;
    let mut rev = 0;
    let mut send = |action, text: &str| -> Result<Reply, Box<dyn std::error::Error>> {
        op += 1;
        if action == Action::Update {
            rev += 1;
        }
        let reply = exchange(pipe, capability, &task, 1, op, rev, action, text, false)?;
        if !reply.applied {
            return Err(format!("unified native {action:?}: {}", reply.code).into());
        }
        Ok(reply)
    };
    send(Action::Begin, "")?;
    send(Action::Update, "周三开会")?;
    drop(send);
    op += 1;
    let mut current = snapshot(request(
        pipe,
        capability,
        &task,
        op,
        BlockCommand {
            lease: 0,
            id: uuid::Uuid::new_v4().to_string(),
            action: BlockAction::Bind,
            epoch: 0,
            version: 0,
            expected: String::new(),
            selection_start: 0,
            selection_end: 0,
        },
        "",
    )?)?;
    for (round, edit_start, edit_end, typed, expected, updates) in [
        (
            0,
            1,
            2,
            "五",
            "周五开会",
            vec![
                "周五开会，不对",
                "周五开会，不对，改成下周一上午开会",
                "下周一上午开会",
            ],
        ),
        (
            1,
            3,
            5,
            "下午",
            "下周一下午开会",
            vec!["下周一下午开会，改成三点", "下周一下午三点开会"],
        ),
        (
            2,
            3,
            5,
            "",
            "下周一三点开会",
            vec!["下周一三点开会，线上", "下周一三点线上开会"],
        ),
        (
            3,
            5,
            5,
            "\r",
            "下周一三点\r线上开会",
            vec!["下周一三点\n线上开会，改成四点", "下周一四点\n线上开会🙂"],
        ),
    ] {
        op += 1;
        let handoff = exchange(
            pipe,
            capability,
            &task,
            1,
            op,
            rev,
            Action::Handoff,
            "",
            false,
        )?;
        if !handoff.applied {
            return Err(format!("unified handoff: {}", handoff.code).into());
        }
        pump(Duration::from_millis(30));
        unsafe {
            let typed: Vec<u16> = typed.encode_utf16().chain(Some(0)).collect();
            SendMessageW(
                editor,
                0xB1,
                Some(WPARAM(edit_start + 1)),
                Some(LPARAM(edit_end + 1)),
            );
            SendMessageW(
                editor,
                0xC2,
                Some(WPARAM(1)),
                Some(LPARAM(typed.as_ptr() as isize)),
            );
            SendMessageW(
                editor,
                0xB1,
                Some(WPARAM(expected.encode_utf16().count() + 1)),
                Some(LPARAM(expected.encode_utf16().count() as isize + 1)),
            );
        }
        pump(Duration::from_millis(40));
        if editor_text(editor)?.replace("\r\n", "\r") != format!("左{expected}右") {
            return Err(format!(
                "unified user edit {round} differs: {:?}",
                editor_text(editor)?
            )
            .into());
        }
        op += 1;
        current = snapshot(request(
            pipe,
            capability,
            &task,
            op,
            command(&current, BlockAction::Read),
            "",
        )?)?;
        if current.text != expected {
            return Err(format!("unified baseline {round} differs").into());
        }
        if round == 0 {
            // Rejected HRESULT, null-object wrapper error, then synchronous
            // revocation with the same prepared listener. Each retry must find
            // no leaked composition listener, and no attempt may change text.
            for mode in [7, 8, 9] {
                unsafe { fault(mode) };
                op += 1;
                let rejected = request(
                    pipe,
                    capability,
                    &task,
                    op,
                    command(&current, BlockAction::Acquire),
                    "",
                )?;
                if rejected.applied || !rejected.code.starts_with("blockStartComposition:") {
                    return Err(format!("acquisition failure {mode}: {}", rejected.code).into());
                }
                pump(Duration::from_millis(40));
                if editor_text(editor)? != format!("左{expected}右") {
                    return Err("failed acquisition changed the body".into());
                }
                op += 1;
                current = snapshot(request(
                    pipe,
                    capability,
                    &task,
                    op,
                    command(&current, BlockAction::Read),
                    "",
                )?)?;
            }
        }
        op += 1;
        current = snapshot(request(
            pipe,
            capability,
            &task,
            op,
            command(&current, BlockAction::Acquire),
            "",
        )?)?;
        rev = op;
        for text in updates {
            op += 1;
            rev += 1;
            let reply = exchange(
                pipe,
                capability,
                &task,
                1,
                op,
                rev,
                Action::Update,
                text,
                false,
            )?;
            if !reply.applied {
                return Err(format!("unified write {round}: {}", reply.code).into());
            }
            for delay in [0, 30, 70] {
                pump(Duration::from_millis(delay));
                // These fixed requests contain no literal U+2028. The shared
                // writer marks only their requested line breaks until handoff.
                let actual = editor_text(editor)?
                    .replace("\r\n", "\n")
                    .replace(['\r', '\u{2028}'], "\n");
                if actual != format!("左{text}右") {
                    return Err(format!(
                        "unified post-event body {round} delay {delay} differs: {:?}",
                        editor_text(editor)?
                    )
                    .into());
                }
            }
        }
        println!("unified block round={round} actual_body_checked=true");
    }
    if std::env::var("SAYWEAVE_BLOCK_FOLLOW_FAILURE").as_deref() == Ok("1") {
        unsafe {
            fault(10);
        }
        op += 1;
        rev += 1;
        let failed = exchange(
            pipe,
            capability,
            &task,
            1,
            op,
            rev,
            Action::Update,
            "已经发生的写入",
            false,
        )?;
        if failed.applied || failed.mutation == Mutation::NotAttempted {
            return Err("post-write guard failure falsely confirmed or lost its effect".into());
        }
        pump(Duration::from_millis(80));
        if editor_text(editor)? != "左已经发生的写入右" {
            return Err("post-write failure body differs".into());
        }
        op += 1;
        let refused = request(
            pipe,
            capability,
            &task,
            op,
            command(&current, BlockAction::Read),
            "",
        )?;
        if refused.applied || editor_text(editor)? != "左已经发生的写入右" {
            return Err("failed writer revived or replayed content".into());
        }
        op += 1;
        let _ = exchange(
            pipe,
            capability,
            &task,
            1,
            op,
            rev,
            Action::Handoff,
            "",
            false,
        )?;
        op += 1;
        if !request(
            pipe,
            capability,
            &task,
            op,
            command(&current, BlockAction::Release),
            "",
        )?
        .applied
        {
            return Err("failed writer tracking did not retire".into());
        }
        println!("post-write guard failure preserves effect, rejects replay and retires=true");
        return Ok(());
    }
    op += 1;
    let done = exchange(
        pipe,
        capability,
        &task,
        1,
        op,
        rev,
        Action::Handoff,
        "",
        false,
    )?;
    if !done.applied {
        return Err("unified final handoff failed".into());
    }
    op += 1;
    if !request(
        pipe,
        capability,
        &task,
        op,
        command(&current, BlockAction::Release),
        "",
    )?
    .applied
    {
        return Err("unified tracking release failed".into());
    }
    pump(Duration::from_millis(100));
    if editor_text(editor)?
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        != "左下周一四点\n线上开会🙂右"
    {
        return Err("unified final body differs".into());
    }
    Ok(())
}
