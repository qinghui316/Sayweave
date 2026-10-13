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
        revision: 1,
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
    if std::env::var("SAYWEAVE_BLOCK_FIXTURE").as_deref() == Ok("unified") {
        return unified(editor, pipe, capability);
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
    let renewed = snapshot(request(
        pipe,
        capability,
        &task,
        11,
        command(&current, BlockAction::Acquire),
        "",
    )?)?;
    if lease.lease == renewed.lease {
        return Err("acquire reused write lease".into());
    }
    let obsolete = request(
        pipe,
        capability,
        &task,
        12,
        command(&lease, BlockAction::Apply),
        "过期正文",
    )?;
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
        command(&renewed, BlockAction::Apply),
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
        24,
        command(&applied, BlockAction::Acquire),
        "",
    )?)?;
    let multiline = "周四下午两点开会\n下一行 👩‍💻 e\u{301}";
    let applied = snapshot(request(
        pipe,
        capability,
        &task,
        25,
        command(&acquired, BlockAction::Apply),
        multiline,
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
        26,
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
        27,
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
        9,
        command(&applied, BlockAction::Read),
        "",
    )?;
    let write = request(
        pipe,
        capability,
        &task,
        10,
        command(&applied, BlockAction::Apply),
        "不能写入",
    )?;
    if denied.applied
        || write.applied
        || write.mutation != Mutation::NotAttempted
        || editor_text(editor)? != "改下周一上午开会右"
    {
        return Err("outside edit retained block write authority".into());
    }
    if !request(
        pipe,
        capability,
        &task,
        11,
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
    Request {
        protocol: PROTOCOL,
        engine: fixture_binding(),
        capability: capability.into(),
        task: task.into(),
        turn: 1,
        operation,
        revision: 1,
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
    for (kind, op) in [(3, 13), (4, 15)] {
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
        17,
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
        18,
        None,
        Action::Revoke,
        "",
        Some(Revoke {
            operation: 17,
            revision: 1,
        }),
    ))?)?;
    let (original, revoke) = pair(pipe, 17, 18)?;
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
        19,
        command(&current, BlockAction::Read),
        "",
    )?)?;
    current = snapshot(request(
        pipe,
        capability,
        task,
        20,
        command(&current, BlockAction::Acquire),
        "",
    )?)?;
    unsafe {
        fault(2);
    }
    pipe.send(&serde_json::to_vec(&frame(
        capability,
        task,
        21,
        Some(command(&current, BlockAction::Apply)),
        Action::Inspect,
        "周四下午两点开会",
        None,
    ))?)?;
    let (original, revoke) = pair(pipe, 21, 22)?;
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
        23,
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
fn unified(editor: HWND, pipe: &Pipe, capability: &str) -> Result<(), Box<dyn std::error::Error>> {
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
        op += 1;
        current = snapshot(request(
            pipe,
            capability,
            &task,
            op,
            command(&current, BlockAction::Acquire),
            "",
        )?)?;
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
