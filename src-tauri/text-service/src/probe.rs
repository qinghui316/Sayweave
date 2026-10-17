//! Stage-A operator probe. Not a renderer command and not part of the production dictation path.
use sayweave_text_service::{registration, transport::Pipe, wire::*};
use std::time::{Duration, Instant};
use windows::{
    Win32::{
        System::Com::*,
        UI::{TextServices::*, WindowsAndMessaging::*},
    },
    core::Interface,
};
fn fixture_binding() -> EngineBinding {
    EngineBinding {
        proxy_instance: "11111111-1111-4111-8111-111111111111".into(),
        engine_generation: 1,
    }
}
struct Apartment;
#[cfg(feature = "acceptance")]
#[path = "block_probe.rs"]
mod block_probe;
#[cfg(feature = "acceptance")]
#[path = "core_switch_probe.rs"]
mod core_switch_probe;
#[cfg(feature = "acceptance")]
#[path = "formal_switch_probe.rs"]
mod formal_switch_probe;
#[cfg(feature = "acceptance")]
#[path = "richtext_block_probe.rs"]
mod richtext_block_probe;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("TSF probe failed: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _apartment = Apartment;
        let args: Vec<_> = std::env::args().collect();
        match args.get(1).map(String::as_str) {
            #[cfg(feature = "acceptance")]
            Some("formal-core-switch") => formal_switch_probe::run(&args)?,
            #[cfg(feature = "acceptance")]
            Some("richtext-block-script") => richtext_block_probe::run(&args)?,
            #[cfg(feature = "acceptance")]
            Some("core-switch") => core_switch_probe::run(&args)?,
            #[cfg(feature = "acceptance")]
            Some("core-switch-multi") => core_switch_probe::multi(&args)?,
            #[cfg(feature = "acceptance")]
            Some("host-identity-hold") => {
                let capability = uuid::Uuid::new_v4().to_string();
                let host = registration::host_capability(std::process::id(), &capability)?;
                if registration::host_capability(std::process::id(), &capability).is_ok()
                    || registration::read_host() != Some((std::process::id(), capability))
                {
                    return Err("host identity/reentry guard failed".into());
                }
                println!("host_identity_ready pid={}", std::process::id());
                pump(Duration::from_secs(20));
                drop(host);
                if registration::read_host().is_some() {
                    return Err("released host identity remained visible".into());
                }
                println!("host_identity_cleanup=true");
            }
            #[cfg(feature = "acceptance")]
            Some("host-identity-absent") => {
                if registration::read_host().is_some() {
                    return Err("inactive host identity remained visible".into());
                }
                println!("host_identity_absent=true");
            }
            #[cfg(feature = "acceptance")]
            Some("host-identity-read") => {
                let expected: u32 = args.get(2).ok_or("expected PID required")?.parse()?;
                let actual = registration::read_host().map(|p| p.0);
                println!("host_identity_match={}", actual == Some(expected));
                if actual != Some(expected) {
                    return Err("host identity was not visible in reader context".into());
                }
            }
            #[cfg(feature = "acceptance")]
            Some("fixture") => fixture()?,
            #[cfg(feature = "acceptance")]
            Some("cross-process-fixture") => cross_process_fixture()?,
            #[cfg(feature = "acceptance")]
            Some("installed-fixture") => fixture()?,
            #[cfg(feature = "acceptance")]
            Some("capabilities") => println!(
                "{{\"acceptance\":true,\"installedFixture\":true,\"protocol\":{}}}",
                sayweave_text_service::wire::PROTOCOL
            ),
            #[cfg(feature = "acceptance")]
            Some("registration-query-detail") => {
                println!("{}", registration_query_detail()?);
            }
            #[cfg(feature = "acceptance")]
            Some("registration-removal-diagnosis") => {
                registration_removal_diagnosis(args.get(2).ok_or("DLL path required")?)?;
            }
            #[cfg(feature = "acceptance")]
            Some("registration-state") => {
                let all = input_profiles()?;
                for profile in all.iter().filter(|p| p.clsid == registration::CLASS) {
                    eprintln!(
                        "owned profile language={} type={} profile={:?} category={:?} flags={}",
                        profile.langid,
                        profile.dwProfileType,
                        profile.guidProfile,
                        profile.catid,
                        profile.dwFlags
                    );
                }
                let profiles = all
                    .iter()
                    .filter(|p| p.clsid == registration::CLASS)
                    .count();
                let manager: ITfCategoryMgr =
                    CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
                let enumerator = manager.EnumCategoriesInItem(&registration::CLASS)?;
                let mut categories = 0;
                let mut speech = false;
                loop {
                    let mut batch = [windows::core::GUID::default(); 16];
                    let mut fetched = 0;
                    enumerator.Next(&mut batch, Some(&mut fetched)).ok()?;
                    if fetched as usize > batch.len() || categories + fetched > 512 {
                        return Err("owned category enumeration exceeded bound".into());
                    }
                    categories += fetched;
                    speech |= batch[..fetched as usize].contains(&GUID_TFCAT_TIP_SPEECH);
                    if fetched == 0 {
                        break;
                    }
                }
                println!(
                    "{{\"profiles\":{profiles},\"categories\":{categories},\"speechCategory\":{speech}}}"
                );
            }
            #[cfg(feature = "acceptance")]
            Some("observation-script") => {
                observation_script(args.get(2).ok_or("owned PID required")?.parse()?)?;
            }
            Some("register") => {
                let path = std::fs::canonicalize(args.get(2).ok_or("DLL path required")?)?;
                registration::register(&path.to_string_lossy())?;
                println!("Registered current-user speech profile; default keyboard unchanged.");
            }
            Some("register-machine") => {
                registration::register_machine(args.get(2).ok_or("DLL path required")?)?;
                println!("Installed speech component; activation remains operator-controlled.");
            }
            Some("remove-machine") => {
                registration::unregister_machine()?;
                println!("Removed owned machine speech component.");
            }
            #[cfg(feature = "acceptance")]
            Some("disable-registered-profile") => {
                let owned: Vec<_> = input_profiles()?
                    .into_iter()
                    .filter(|profile| profile.clsid == registration::CLASS)
                    .collect();
                if owned.len() != 1 || owned[0].guidProfile != registration::PROFILE {
                    return Err("fixed fixture profile identity differs".into());
                }
                registration::profiles()?.EnableLanguageProfile(
                    &registration::CLASS,
                    owned[0].langid,
                    &registration::PROFILE,
                    false,
                )?;
                println!("Disabled exact fixture profile; registration retained.");
            }
            #[cfg(feature = "acceptance")]
            Some("repair-machine-metadata") => {
                registration::acceptance_replace_machine(
                    args.get(2).ok_or("replacement path required")?,
                    args.get(3).ok_or("expected owned path required")?,
                    args.get(4)
                        .is_some_and(|value| value == "reject-confirmation"),
                )?;
                println!("Owned component metadata repaired; profile relationships retained.");
            }
            #[cfg(feature = "acceptance")]
            Some("registration-rollback") => {
                let path = std::fs::canonicalize(args.get(2).ok_or("DLL path required")?)?;
                registration::acceptance_registration_rollback(&path.to_string_lossy())?;
            }
            Some("remove") => {
                registration::unregister()?;
                println!("Removed Sayweave speech profile.");
            }
            Some("probe" | "activation-check") => {
                let capability = uuid::Uuid::new_v4().to_string();
                let _host = registration::host_capability(std::process::id(), &capability)?;
                let keyboard_before = keyboard()?;
                let activation_before = sayweave_text_service::client::speech_activation()?;
                let mut host = sayweave_text_service::client::ProfileLease::open()?;
                if args[1] == "activation-check" {
                    host.close()?;
                    if keyboard()? != keyboard_before {
                        return Err("keyboard profile changed during cleanup".into());
                    }
                    if sayweave_text_service::client::speech_activation()? != activation_before {
                        return Err("speech activation was not restored".into());
                    }
                    println!(
                        "speech lease and prior activation restored ({:?}); keyboard unchanged; no input attempted",
                        activation_before.0
                    );
                    return Ok(());
                }
                let focus_seconds = args
                    .get(3)
                    .map(|s| s.parse::<u64>())
                    .transpose()?
                    .unwrap_or(5);
                let owned_immediate = focus_seconds == 0
                    && std::env::var("SAYWEAVE_INPUT_ACCEPTANCE").as_deref() == Ok("1")
                    && args
                        .get(2)
                        .and_then(|pid| pid.parse::<u32>().ok())
                        .is_some();
                if !owned_immediate && !(5..=30).contains(&focus_seconds) {
                    return Err("operator focus countdown must be between5and30seconds".into());
                }
                println!(
                    "Focus the intended blank input within{focus_seconds}seconds. This probe writes two cumulative snapshots, then commits once."
                );
                pump(Duration::from_secs(focus_seconds));
                let window = GetForegroundWindow();
                let mut pid = 0;
                let thread = GetWindowThreadProcessId(window, Some(&mut pid));
                if let Some(expected) = args.get(2)
                    && pid != expected.parse::<u32>()?
                {
                    return Err("foreground is not the owned fixture; no input attempted".into());
                }
                if pid == std::process::id() {
                    return Err("probe cannot target its own window".into());
                }
                let start = Instant::now();
                let pipe = loop {
                    if let Ok(pipe) = Pipe::client(pid, thread) {
                        break pipe;
                    }
                    if start.elapsed() > Duration::from_secs(3) {
                        return Err(
                            "speech profile activated but target thread has no TSF bridge".into(),
                        );
                    }
                    pump(Duration::from_millis(50));
                };
                let task = uuid::Uuid::new_v4().to_string();
                let multiline = args.get(4).map(String::as_str) == Some("multiline");
                let prefix_probe = matches!(
                    args.get(4).map(String::as_str),
                    Some(
                        "prefix"
                            | "prefix-multiline"
                            | "prefix-gap"
                            | "prefix-gap-abort"
                            | "prefix-abort-before"
                            | "prefix-abort-after"
                    )
                );
                let prefix_gap = matches!(
                    args.get(4).map(String::as_str),
                    Some(
                        "prefix-gap"
                            | "prefix-gap-abort"
                            | "prefix-abort-before"
                            | "prefix-abort-after"
                    )
                );
                let prefix_abort = args.get(4).map(String::as_str) == Some("prefix-gap-abort");
                let prefix_multiline = args.get(4).map(String::as_str) == Some("prefix-multiline");
                let interval = args
                    .get(5)
                    .map(|s| s.parse::<u64>())
                    .transpose()?
                    .unwrap_or(700);
                // Operator observation delay only; native request deadlines and
                // production recording/processing latency are unchanged.
                if interval > 3000 {
                    return Err("probe interval exceeds limit".into());
                }
                let begin_interval = args
                    .get(6)
                    .map(|s| s.parse::<u64>())
                    .transpose()?
                    .unwrap_or(interval);
                let first_write_interval = args
                    .get(7)
                    .map(|s| s.parse::<u64>())
                    .transpose()?
                    .unwrap_or(interval);
                if (args.len() > 6 && !owned_immediate)
                    || begin_interval > 3000
                    || first_write_interval > 3000
                {
                    return Err("independent intervals require a bounded owned fixture".into());
                }
                let first_boundary =
                    sayweave_text_service::settlement_boundary::SettlementBoundary::prepare(
                        "",
                        if prefix_multiline {
                            "第一段。\n"
                        } else {
                            "第一段。"
                        },
                    )
                    .unwrap();
                let second_boundary =
                    sayweave_text_service::settlement_boundary::SettlementBoundary::prepare(
                        &first_boundary.retained_separator,
                        if prefix_multiline {
                            "第二段。\n"
                        } else {
                            "第二段。"
                        },
                    )
                    .unwrap();
                let first_tail =
                    first_boundary.tail(if prefix_gap { "" } else { "第二段临时" });
                let growing_tail = first_boundary.tail("第二段续写🙂");
                let second_tail = second_boundary.tail(if prefix_gap { "" } else { "尾部🙂" });
                let final_tail = second_boundary.tail("尾部完成🙂");
                let mut steps = if prefix_probe {
                    vec![
                        (1, 0, Action::Inspect, "", None),
                        (2, 0, Action::Begin, "", None),
                        (
                            3,
                            1,
                            Action::Update,
                            if prefix_gap {
                                "ASR 第一段"
                            } else {
                                "ASR 第一段，第二段临时"
                            },
                            None,
                        ),
                        (
                            4,
                            2,
                            Action::Settle,
                            first_tail.as_str(),
                            Some(first_boundary.committed.as_str()),
                        ),
                        (5, 3, Action::Update, growing_tail.as_str(), None),
                        (
                            6,
                            4,
                            Action::Settle,
                            second_tail.as_str(),
                            Some(second_boundary.committed.as_str()),
                        ),
                        (7, 5, Action::Update, final_tail.as_str(), None),
                        (8, 5, Action::Commit, "", None),
                    ]
                } else {
                    vec![
                        (1, 0, Action::Inspect, ""),
                        (2, 0, Action::Begin, ""),
                        (3, 1, Action::Update, "Sayweave 原位"),
                        (
                            4,
                            2,
                            Action::Update,
                            if multiline {
                                "中文，🙂\n第二行 e\u{0301}。"
                            } else {
                                "Sayweave 原位验证，🙂"
                            },
                        ),
                        (5, 2, Action::Commit, ""),
                    ]
                    .into_iter()
                    .map(|(op, revision, action, text)| (op, revision, action, text, None))
                    .collect()
                };
                if prefix_abort {
                    steps.truncate(5);
                    steps.push((6, 3, Action::Abort, "", None));
                }
                if matches!(
                    args.get(4).map(String::as_str),
                    Some("prefix-abort-before" | "prefix-abort-after")
                ) {
                    if !owned_immediate {
                        return Err("initial abort requires an owned fixture".into());
                    }
                    let before = args[4] == "prefix-abort-before";
                    steps.truncate(if before { 2 } else { 3 });
                    steps.push((
                        if before { 3 } else { 4 },
                        if before { 0 } else { 1 },
                        Action::Abort,
                        "",
                        None,
                    ));
                }
                let mut range_epoch = 0;
                let mut previous_revision = 0;
                for (op, revision, action, text, processed) in steps {
                    if GetForegroundWindow() != window {
                        return Err("target foreground changed; stopped without retry".into());
                    }
                    let r = Request {
                        revoke: None,
                        block: None,
                        engine: fixture_binding(),
                        protocol: sayweave_text_service::wire::PROTOCOL,
                        capability: capability.clone(),
                        task: task.clone(),
                        turn: 1,
                        operation: op,
                        revision,
                        deadline_tick: windows::Win32::System::SystemInformation::GetTickCount64()
                            + 2000,
                        action,
                        text: text.into(),
                        prefix: processed.map(|text| Prefix {
                            text: text.into(),
                            expected_revision: previous_revision,
                            range_epoch,
                        }),
                    };
                    pipe.send(&serde_json::to_vec(&r)?)?;
                    let wait = Instant::now();
                    let reply = loop {
                        if let Some(data) = pipe.receive()? {
                            break serde_json::from_slice::<Reply>(&data)?;
                        }
                        if wait.elapsed() > Duration::from_secs(2) {
                            return Err(
                                "edit acknowledgement timeout; no automatic reinsertion".into()
                            );
                        }
                        pump(Duration::from_millis(10));
                    };
                    println!(
                        "task={task} operation={} revision={} applied={} code={}",
                        reply.operation, reply.revision, reply.applied, reply.code
                    );
                    if reply.operation != op || reply.revision != revision || !reply.applied {
                        return Err("target did not apply edit".into());
                    }
                    if action == Action::Inspect && !reply.compatible_runtime(pid, thread) {
                        return Err("target runtime mismatch; no input attempted".into());
                    }
                    if action == Action::Settle
                        && !reply.settlement.is_some_and(|s| {
                            s.prefix_operation == op
                                && s.prefix == Mutation::Changed
                                && s.tail_owned
                        })
                    {
                        return Err("target did not retain a writable settled tail".into());
                    }
                    range_epoch = reply.range_epoch;
                    previous_revision = revision;
                    pump(Duration::from_millis(match op {
                        2 => begin_interval,
                        3 => first_write_interval,
                        _ => interval,
                    }));
                }
                if keyboard()? != keyboard_before {
                    return Err("keyboard profile changed during probe".into());
                }
                host.close()?;
                if keyboard()? != keyboard_before {
                    return Err("keyboard profile changed during cleanup".into());
                }
                println!(
                    "Composition updates acknowledged. Actual target text still requires visual/readback acceptance."
                );
            }
            Some("status") => {
                for profile in input_profiles()?
                    .iter()
                    .filter(|p| p.clsid == registration::CLASS)
                {
                    println!(
                        "sayweave_speech_profile={} language={} enabled={} active={}",
                        profile.catid == GUID_TFCAT_TIP_SPEECH,
                        profile.langid,
                        registration::profiles()?
                            .IsEnabledLanguageProfile(
                                &profile.clsid,
                                profile.langid,
                                &profile.guidProfile
                            )?
                            .as_bool(),
                        profile.dwFlags & TF_IPP_FLAG_ACTIVE != 0
                    );
                }
            }
            _ => return Err("Usage: tsf-probe register DLL | remove | status | probe".into()),
        }
        Ok(())
    }
}
#[cfg(feature = "acceptance")]
fn observation_script(expected: u32) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{BufRead, Write};
    if expected == 0
        || std::env::var("SAYWEAVE_TSF_OWNED_PID")
            .ok()
            .and_then(|p| p.parse::<u32>().ok())
            != Some(expected)
    {
        return Err("owned fixture identity required".into());
    }
    let capability = uuid::Uuid::new_v4().to_string();
    let _host = registration::host_capability(std::process::id(), &capability)?;
    let before = keyboard()?;
    let mut profile = sayweave_text_service::client::ProfileLease::open()?;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut process = 0;
        let thread = unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut process)) };
        if process != expected {
            return Err("owned foreground mismatch; no write attempted".into());
        }
        let began = Instant::now();
        let pipe = loop {
            if let Ok(pipe) = Pipe::client(process, thread) {
                break pipe;
            }
            if began.elapsed() > Duration::from_secs(3) {
                return Err("owned bridge unavailable".into());
            }
            pump(Duration::from_millis(20));
        };
        let task = uuid::Uuid::new_v4().to_string();
        for (operation, revision, action, text) in [
            (1, 0, Action::Begin, ""),
            (2, 0, Action::ObserveStart, ""),
            (3, 1, Action::Update, "项目名是起蓝。"),
            (4, 1, Action::Commit, ""),
        ] {
            let reply = exchange(
                &pipe,
                &capability,
                &task,
                1,
                operation,
                revision,
                action,
                text,
                false,
            )?;
            if !reply.applied {
                return Err(format!("owned observation initialization: {}", reply.code).into());
            }
        }
        println!("{{\"ready\":true}}");
        std::io::stdout().flush()?;
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else {
                    break;
                };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        let began = Instant::now();
        loop {
            if let Ok(line) = receive.try_recv() {
                let command: serde_json::Value = serde_json::from_str(&line)?;
                if command["action"] != "stop" {
                    return Err("only owned observation stop is supported".into());
                }
                let reply = exchange(
                    &pipe,
                    &capability,
                    &task,
                    1,
                    5,
                    1,
                    Action::ObserveStop,
                    "",
                    false,
                )?;
                println!(
                    "{}",
                    serde_json::json!({"applied":reply.applied,"edit_observation":reply.edit_observation})
                );
                std::io::stdout().flush()?;
                break;
            }
            if began.elapsed() > Duration::from_secs(12) {
                return Err("owned observation deadline".into());
            }
            pump(Duration::from_millis(10));
        }
        Ok(())
    })();
    profile.close()?;
    if keyboard()? != before {
        return Err("owned observation changed keyboard profile".into());
    }
    result
}
#[cfg(feature = "acceptance")]
fn registration_query_detail() -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use windows::Win32::System::Registry::*;
    unsafe {
        let all = input_profiles()?;
        let profile_rows: Vec<_> = all.iter().filter(|p| p.clsid == registration::CLASS).map(|p| {
            serde_json::json!({"clsid":format!("{:?}",p.clsid),"profile":format!("{:?}",p.guidProfile),
                "category":format!("{:?}",p.catid),"language":p.langid,"type":p.dwProfileType,"flags":p.dwFlags})
        }).collect();
        let category: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        let enumerator = category.EnumCategoriesInItem(&registration::CLASS)?;
        let mut category_rows = Vec::new();
        loop {
            let mut batch = [windows::core::GUID::default(); 16];
            let mut fetched = 0;
            enumerator.Next(&mut batch, Some(&mut fetched)).ok()?;
            if fetched as usize > batch.len() || category_rows.len() + fetched as usize > 512 {
                return Err("category inventory exceeded bound".into());
            }
            category_rows.extend(batch[..fetched as usize].iter().map(|g| format!("{g:?}")));
            if fetched == 0 {
                break;
            }
        }
        let profiles = registration::profiles()?;
        let language = profiles.GetCurrentLanguage()?;
        let legacy = profiles.EnumLanguageProfiles(language)?;
        let mut legacy_rows = Vec::new();
        let mut seen = 0;
        loop {
            let mut batch = [TF_LANGUAGEPROFILE::default(); 16];
            let mut fetched = 0;
            legacy.Next(&mut batch, &mut fetched)?;
            if fetched as usize > batch.len() || seen + fetched > 512 {
                return Err("legacy inventory exceeded bound".into());
            }
            seen += fetched;
            legacy_rows.extend(batch[..fetched as usize].iter().filter(|p| p.clsid == registration::CLASS).map(|p| {
                serde_json::json!({"clsid":format!("{:?}",p.clsid),"profile":format!("{:?}",p.guidProfile),
                    "category":format!("{:?}",p.catid),"language":p.langid,"active":p.fActive.as_bool()})
            }));
            if fetched == 0 {
                break;
            }
        }
        let mut roots = serde_json::Map::new();
        for (name, root) in [("user", HKEY_CURRENT_USER), ("machine", HKEY_LOCAL_MACHINE)] {
            let mut state = serde_json::Map::new();
            for (label, path) in [
                (
                    "tip",
                    "Software\\Microsoft\\CTF\\TIP\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}",
                ),
                (
                    "com",
                    "Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}",
                ),
            ] {
                let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
                let mut key = HKEY::default();
                let status = RegOpenKeyExW(
                    root,
                    windows::core::PCWSTR(wide.as_ptr()),
                    None,
                    KEY_READ,
                    &mut key,
                );
                if status == windows::Win32::Foundation::ERROR_SUCCESS {
                    RegCloseKey(key).ok()?;
                    state.insert(label.into(), true.into());
                } else if status == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
                    || status == windows::Win32::Foundation::ERROR_PATH_NOT_FOUND
                {
                    state.insert(label.into(), false.into());
                } else {
                    status.ok()?;
                }
            }
            roots.insert(name.into(), state.into());
        }
        Ok(
            serde_json::json!({"pid":std::process::id(),"class":format!("{:?}",registration::CLASS),
            "expectedProfile":format!("{:?}",registration::PROFILE),"profilesClass":format!("{CLSID_TF_InputProcessorProfiles:?}"),
            "categoryClass":format!("{CLSID_TF_CategoryMgr:?}"),"managerProfiles":profile_rows,
            "categories":category_rows,"legacyLanguage":language,"legacyProfiles":legacy_rows,"roots":roots}),
        )
    }
}
#[cfg(feature = "acceptance")]
fn registration_removal_diagnosis(module: &str) -> Result<(), Box<dyn std::error::Error>> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        || std::env::var("RUNNER_ENVIRONMENT").as_deref() != Ok("github-hosted")
    {
        return Err("requires disposable GitHub-hosted runner".into());
    }
    let module = std::fs::canonicalize(module)?;
    if std::fs::metadata(&module)?.len() > 32 * 1024 * 1024 {
        return Err("module exceeds bound".into());
    }
    let bytes = std::fs::read(&module)?;
    println!(
        "{}",
        serde_json::json!({"phase":"before","module":module,"sha256":format!("{:x}",Sha256::digest(bytes)),"state":registration_query_detail()?})
    );
    std::io::stdout().flush()?;
    let start = Instant::now();
    let removed = registration::unregister();
    println!(
        "{}",
        serde_json::json!({"phase":"remove","elapsedMs":start.elapsed().as_millis(),"result":removed.as_ref().map(|_| "ok").map_err(|e| e.to_string())})
    );
    std::io::stdout().flush()?;
    for (phase, sleep) in [
        ("immediate", 0),
        ("noPump100", 100),
        ("noPump500", 400),
        ("noPump1500", 1000),
        ("beforePump1500", 0),
        ("pump2500", 1000),
        ("pump4500", 2000),
    ] {
        if phase.starts_with("pump") {
            pump(Duration::from_millis(sleep));
        } else {
            std::thread::sleep(Duration::from_millis(sleep));
        }
        let current = registration_query_detail()?;
        let mut child = std::process::Command::new(std::env::current_exe()?);
        use std::os::windows::process::CommandExt;
        child.creation_flags(0x08000000);
        let independent = child.arg("registration-query-detail").output()?;
        if independent.stdout.len() > 131072 || independent.stderr.len() > 8192 {
            return Err("child query exceeded bound".into());
        }
        let child_state: serde_json::Value = if independent.status.success() {
            serde_json::from_slice(&independent.stdout)?
        } else {
            serde_json::json!({"exit":independent.status.code(),"stderr":String::from_utf8_lossy(&independent.stderr)})
        };
        println!(
            "{}",
            serde_json::json!({"phase":phase,"elapsedMs":start.elapsed().as_millis(),"currentNewCom":current,"independentProcess":child_state})
        );
        std::io::stdout().flush()?;
    }
    Ok(())
}
fn input_profiles() -> windows::core::Result<Vec<TF_INPUTPROCESSORPROFILE>> {
    unsafe {
        let manager: ITfInputProcessorProfileMgr = registration::profiles()?.cast()?;
        let enumerator = manager.EnumProfiles(0)?;
        let mut result = Vec::new();
        loop {
            let mut batch = [TF_INPUTPROCESSORPROFILE::default(); 16];
            let mut fetched = 0;
            enumerator.Next(&mut batch, &mut fetched)?;
            if fetched as usize > batch.len() || result.len() + fetched as usize > 512 {
                return Err(windows::core::Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    "profile enumeration exceeded bounds",
                ));
            }
            result.extend_from_slice(&batch[..fetched as usize]);
            if fetched == 0 {
                return Ok(result);
            }
        }
    }
}
fn keyboard() -> windows::core::Result<(u32, u16, windows::core::GUID, windows::core::GUID, isize)>
{
    unsafe {
        let manager: ITfInputProcessorProfileMgr = registration::profiles()?.cast()?;
        let mut profile = TF_INPUTPROCESSORPROFILE::default();
        manager.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &mut profile)?;
        Ok((
            profile.dwProfileType,
            profile.langid,
            profile.clsid,
            profile.guidProfile,
            profile.hkl.0 as isize,
        ))
    }
}
#[cfg(feature = "acceptance")]
fn cross_process_fixture() -> Result<(), Box<dyn std::error::Error>> {
    use windows::{Win32::UI::WindowsAndMessaging::*, core::w};
    unsafe {
        let top = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("Sayweave owned cross-process editor"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            20,
            20,
            520,
            240,
            None,
            None,
            None,
            None,
        )?;
        let outcome = (|| {
            let _ = SetForegroundWindow(top);
            let dll = std::env::args().nth(2).ok_or("candidate DLL required")?;
            let mut child = std::process::Command::new(std::env::current_exe()?)
                .arg("fixture")
                .arg(dll)
                .env("SAYWEAVE_OWNED_PARENT_WINDOW", (top.0 as usize).to_string())
                .env(
                    "SAYWEAVE_OWNED_PARENT_PROCESS",
                    std::process::id().to_string(),
                )
                .spawn()?;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                pump(Duration::from_millis(10));
                if let Some(status) = child.try_wait()? {
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(format!("cross-process editor failed: {status}").into())
                    };
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("owned cross-process fixture deadline".into());
                }
            }
        })();
        let _ = DestroyWindow(top);
        outcome
    }
}
#[cfg(feature = "acceptance")]
fn fixture() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        use windows::{
            Win32::{
                Foundation::*,
                System::{LibraryLoader::*, Threading::GetCurrentThreadId},
                UI::Input::KeyboardAndMouse::*,
            },
            core::w,
        };
        let installed = std::env::args().nth(1).as_deref() == Some("installed-fixture");
        if installed && std::env::args().nth(2).is_none() {
            return Err("installed fixture requires an explicit production DLL".into());
        }
        let keyboard_before = if installed { Some(keyboard()?) } else { None };
        let plain = installed || std::env::args().nth(3).as_deref() == Some("plain");
        println!("owned fixture stage=library");
        let library = LoadLibraryExW(w!("Msftedit.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)?;
        println!("owned fixture stage=window");
        let external_parent = std::env::var("SAYWEAVE_OWNED_PARENT_WINDOW")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .zip(
                std::env::var("SAYWEAVE_OWNED_PARENT_PROCESS")
                    .ok()
                    .and_then(|v| v.parse::<u32>().ok()),
            );
        let top = if let Some((window, expected)) = external_parent {
            let window = HWND(window as *mut _);
            let mut actual = 0;
            GetWindowThreadProcessId(window, Some(&mut actual));
            if actual != expected
                || actual == std::process::id()
                || !IsWindow(Some(window)).as_bool()
                || GetAncestor(window, GA_ROOT) != window
            {
                return Err("owned parent identity mismatch".into());
            }
            println!(
                "owned fixture separate root/editor processes=true root={actual} editor={}",
                std::process::id()
            );
            window
        } else {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Sayweave TSF isolated fixture"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                20,
                20,
                520,
                240,
                None,
                None,
                None,
                None,
            )?
        };
        println!("owned fixture stage=editor");
        let editor = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            if plain { w!("EDIT") } else { w!("RICHEDIT50W") },
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_MULTILINE as u32),
            10,
            10,
            480,
            170,
            Some(top),
            None,
            None,
            None,
        )?;
        println!("owned fixture stage=manager");
        let manager: ITfThreadMgr =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("owned manager create: {e}"))?;
        manager
            .Activate()
            .map_err(|e| format!("owned manager activate: {e}"))?;
        println!("owned fixture stage=manager-active");
        if !plain {
            let _ = SendMessageW(
                editor,
                WM_USER + 204,
                Some(WPARAM(0x10000)),
                Some(LPARAM(0x10000)),
            );
        }
        let _ = SetForegroundWindow(top);
        let _ = SetFocus(Some(editor));
        // Test-only operator/controller lease. Allows the desktop adapter to
        // focus this exact newly-created window; no production capture delay.
        if std::env::var_os("SAYWEAVE_TSF_FIXTURE_SUPERVISED").is_some() {
            println!("owned fixture awaiting controller foreground=true");
            let deadline = Instant::now() + Duration::from_secs(30);
            while GetForegroundWindow() != top && Instant::now() < deadline {
                pump(Duration::from_millis(25));
            }
        }
        pump(Duration::from_millis(200));
        println!(
            "owned fixture stage=focus foreground_matches={}",
            GetForegroundWindow() == top
        );
        if GetForegroundWindow() != top {
            println!(
                "owned fixture focus fallback top_valid={} editor_valid={}",
                IsWindow(Some(top)).as_bool(),
                IsWindow(Some(editor)).as_bool()
            );
            // A background test launcher may be denied SetForegroundWindow.
            // Exercise only the blank window this fixture owns, then verify focus.
            SetWindowPos(
                top,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .map_err(|e| format!("owned focus topmost: {e}"))?;
            let mut rect = RECT::default();
            GetWindowRect(editor, &mut rect).map_err(|e| format!("owned focus rect: {e}"))?;
            SetCursorPos(rect.left + 30, rect.top + 30)
                .map_err(|e| format!("owned focus cursor: {e}"))?;
            let events = [MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP].map(|flags| INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: flags,
                        ..Default::default()
                    },
                },
            });
            if SendInput(&events, std::mem::size_of::<INPUT>() as i32) != 2 {
                return Err("owned fixture focus click was not submitted".into());
            }
            pump(Duration::from_millis(200));
            SetWindowPos(
                top,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )?;
            println!("owned fixture required a real activation click=true");
        }
        if GetForegroundWindow() != top {
            return Err("owned fixture is not the actual foreground window".into());
        }
        if let (Some((_, root_process)), Ok(checker)) =
            (external_parent, std::env::var("SAYWEAVE_ENDPOINT_CHECKER"))
        {
            let status = std::process::Command::new(checker)
                .arg("endpoint")
                .arg(root_process.to_string())
                .arg(std::process::id().to_string())
                .env("SAYWEAVE_INPUT_ACCEPTANCE", "1")
                .status()?;
            if !status.success() {
                return Err("desktop endpoint verification failed".into());
            }
        }
        let module_path = std::env::args()
            .nth(2)
            .map(std::path::PathBuf::from)
            .unwrap_or(std::env::current_exe()?.with_file_name("sayweave_text_service.dll"));
        // The installed speech profile can already be active in this fixture process.
        // Isolate the explicitly loaded component here only; leave the desktop/user
        // profile and keyboard unchanged.
        println!("owned fixture stage=activation");
        let (activation, language) = sayweave_text_service::client::speech_activation()?;
        if installed && activation == sayweave_text_service::client::SpeechActivation::Other {
            return Err("owned fixture found a conflicting speech profile".into());
        }
        if activation == sayweave_text_service::client::SpeechActivation::Own {
            registration::deactivate(language, false)?;
            pump(Duration::from_millis(100));
        }
        println!("owned fixture stage=component");
        let (component, processor) = if installed {
            (load_fixture_module(&module_path)?, None)
        } else {
            let (module, processor) = load_processor(&module_path)?;
            (module, Some(processor))
        };
        let capability = uuid::Uuid::new_v4().to_string();
        let host = registration::host_capability(std::process::id(), &capability)?;
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            if installed {
                // Only TSF owns a registered TIP's activation identity. Hold the
                // exact module for export/unload checks; TSF creates the service.
                println!("owned fixture stage=service process_scope=true");
                let activated = registration::activate(false)
                    .map_err(|e| format!("owned process TIP activate: {e}"))?;
                if activated != language
                    || !sayweave_text_service::client::own_profile_active(language)?
                    || sayweave_text_service::client::speech_activation()?
                        != (
                            sayweave_text_service::client::SpeechActivation::Own,
                            language,
                        )
                {
                    return Err("owned process TIP activation identity mismatch".into());
                }
                println!(
                    "owned process TIP activation identity confirmed=true language={language}"
                );
                if keyboard_before != Some(keyboard()?) {
                    return Err("owned process TIP activation changed keyboard identity".into());
                }
                pump(Duration::from_millis(200));
            } else {
                let service_client = manager
                    .cast::<ITfClientId>()?
                    .GetClientId(&registration::CLASS)?;
                println!("owned fixture stage=service client={service_client} direct=true");
                processor
                    .as_ref()
                    .ok_or("missing direct fixture processor")?
                    .Activate(&manager, service_client)
                    .map_err(|e| {
                        format!("owned processor activate client={service_client}: {e}")
                    })?;
            }
            if installed {
                if GetProcAddress(component, windows::core::s!("SayweaveSettleFixtureFault"))
                    .is_some()
                    || GetProcAddress(component, windows::core::s!("SayweaveInitialFixtureFault"))
                        .is_some()
                {
                    return Err("installed production DLL exposes a test fault hook".into());
                }
                println!("owned production DLL fault export absent=true");
            }
            let context = manager.GetFocus()?.GetTop()?;
            println!(
                "owned TSF context available; readonly={}",
                context.GetStatus()?.dwDynamicFlags & TF_SD_READONLY != 0
            );
            let pipe = Pipe::client(std::process::id(), GetCurrentThreadId())?;
            if !installed && std::env::var_os("SAYWEAVE_BLOCK_FIXTURE").is_some() {
                return block_probe::run(editor, &manager, &pipe, &capability, component);
            }
            if plain && !installed && std::env::var_os("SAYWEAVE_EDIT_SETTLEMENT_TRACE").is_some() {
                return trace_native_initial_settlement(editor, &context, &pipe, &capability);
            }
            let task = uuid::Uuid::new_v4().to_string();
            let mut expected = String::new();
            let observation =
                !installed && std::env::var_os("SAYWEAVE_TSF_OBSERVATION_ONLY").is_some();
            for (operation, revision, action, text) in [
                (1, 0, Action::Begin, ""),
                (2, 1, Action::Update, "Sayweave 原位"),
                (3, 2, Action::Update, "Sayweave 原位验证，🙂"),
                (4, 2, Action::Commit, ""),
            ] {
                let operation = operation + u64::from(observation && operation > 1);
                let request = Request {
                    revoke: None,
                    block: None,
                    engine: fixture_binding(),
                    protocol: sayweave_text_service::wire::PROTOCOL,
                    capability: capability.clone(),
                    task: task.clone(),
                    turn: 1,
                    operation,
                    revision,
                    deadline_tick: windows::Win32::System::SystemInformation::GetTickCount64()
                        + 2000,
                    action,
                    text: text.into(),
                    prefix: None,
                };
                pipe.send(&serde_json::to_vec(&request)?)?;
                let started = Instant::now();
                let reply = loop {
                    pump(Duration::from_millis(10));
                    if let Some(data) = pipe.receive()? {
                        break serde_json::from_slice::<Reply>(&data)?;
                    }
                    if started.elapsed() > Duration::from_secs(2) {
                        return Err("owned edit acknowledgement timeout".into());
                    }
                };
                if reply.operation != operation || !reply.applied {
                    return Err(format!("owned edit rejected: {}", reply.code).into());
                }
                if observation && action == Action::Begin {
                    let watch = exchange(
                        &pipe,
                        &capability,
                        &task,
                        1,
                        2,
                        0,
                        Action::ObserveStart,
                        "",
                        false,
                    )?;
                    if !watch.applied
                        || watch.edit_observation.as_ref().map(|o| o.capability) != Some(2)
                    {
                        return Err("optional observation capability unavailable".into());
                    }
                }
                if action == Action::Update {
                    expected = text.into();
                }
                // A legacy IMM-backed EDIT keeps active composition outside its
                // committed WM_GETTEXT buffer. Read the actual TSF range while
                // composing, and require real control text after Commit.
                let composing = plain && action == Action::Update;
                let actual = if composing {
                    composition_text(&context)?
                } else {
                    editor_text(editor)?
                };
                if actual != expected {
                    return Err(format!("owned target text assertion failed: operation={operation} expected_utf16={} actual_utf16={} actual_hash={}", expected.encode_utf16().count(), actual.encode_utf16().count(), sayweave_text_service::deployment::version(actual.as_bytes())).into());
                }
                println!(
                    "owned operation={operation} applied=true actual_text_assertion=true source={}",
                    if composing {
                        "compositionRange"
                    } else {
                        "controlText"
                    }
                );
            }
            // Installed production DLLs deliberately omit fault-injection exports.
            // Exercise their actual ordinary protocol and control text, then use
            // the shared COM/lease cleanup below; leave fault tests to fixture.
            if installed {
                prefix_fixture(editor, &manager, &pipe, &capability)?;
                println!("owned installed production DLL actual text=true protocol8=true");
                return Ok(());
            }
            if observation {
                // Observer installation is deferred until the write callback returns.
                pump(Duration::from_millis(120));
                let foreign =
                    manager
                        .cast::<ITfClientId>()?
                        .GetClientId(&windows::core::GUID::from_u128(
                            0x9da150371765447aa05db6c9bb370a60,
                        ))?;
                if plain {
                    SendMessageW(editor, 0x00B1, Some(WPARAM(11)), Some(LPARAM(13)));
                    let keys = "更正"
                        .encode_utf16()
                        .flat_map(|unit| {
                            [false, true].map(move |up| INPUT {
                                r#type: INPUT_KEYBOARD,
                                Anonymous: INPUT_0 {
                                    ki: KEYBDINPUT {
                                        wScan: unit,
                                        dwFlags: KEYEVENTF_UNICODE
                                            | if up {
                                                KEYEVENTF_KEYUP
                                            } else {
                                                KEYBD_EVENT_FLAGS(0)
                                            },
                                        ..Default::default()
                                    },
                                },
                            })
                        })
                        .collect::<Vec<_>>();
                    if SendInput(&keys, std::mem::size_of::<INPUT>() as i32) != keys.len() as u32 {
                        return Err("fixture edit input incomplete".into());
                    }
                } else {
                    let edit: ITfEditSession = UserTextEdit {
                        context: context.clone(),
                        start: 11,
                        length: 2,
                        text: "更正".into(),
                    }
                    .into();
                    context
                        .RequestEditSession(foreign, &edit, TF_ES_SYNC | TF_ES_READWRITE)?
                        .ok()?;
                }
                pump(Duration::from_millis(30));
                if editor_text(editor)? != "Sayweave 原位更正，🙂" {
                    return Err("observed edit target text assertion failed".into());
                }
                let read = exchange(
                    &pipe,
                    &capability,
                    &task,
                    1,
                    6,
                    2,
                    Action::ObserveStop,
                    "",
                    false,
                )?;
                let records = read
                    .edit_observation
                    .ok_or("missing observation payload")?
                    .records;
                if records.len() != 1
                    || records[0].before != "Sayweave 原位验证，🙂"
                    || records[0].after != "Sayweave 原位更正，🙂"
                    || !records[0].ended
                    || records[0].operation != 5
                {
                    return Err(format!(
                        "observed receipt mismatch count={} before_units={} after_units={}",
                        records.len(),
                        records
                            .first()
                            .map_or(0, |r| r.before.encode_utf16().count()),
                        records
                            .first()
                            .map_or(0, |r| r.after.encode_utf16().count())
                    )
                    .into());
                }
                if read.mutation != Mutation::NotAttempted {
                    return Err("observation attempted mutation".into());
                }
                println!("owned receipt observation before_after_actual_text=true read_only=true");
                observation_ranges_fixture(
                    (component, editor),
                    &context,
                    &manager,
                    &pipe,
                    &capability,
                    foreign,
                    plain,
                )?;
                return Ok(());
            }
            prefix_fixture(editor, &manager, &pipe, &capability)?;
            prefix_fault_fixture(component, editor, &pipe, &capability)?;
            if !plain && std::env::var("SAYWEAVE_TSF_SELECTION_REPAIR").as_deref() == Ok("1") {
                initial_selection_fixture(component, editor, &pipe, &capability)?;
            }
            if plain {
                prepared_native_selection_fixture(editor, &pipe, &capability)?;
                let end = editor_text(editor)?.encode_utf16().count();
                SendMessageW(
                    editor,
                    0x00B1,
                    Some(WPARAM(end)),
                    Some(LPARAM(end as isize)),
                );
                pump(Duration::from_millis(20));
                println!(
                    "owned plain EDIT cumulative replacement and single commit actual text=true"
                );
                let before = editor_text(editor)?;
                if !exchange(&pipe, &capability, &task, 2, 1, 0, Action::Begin, "", false)?.applied
                    || !exchange(
                        &pipe,
                        &capability,
                        &task,
                        2,
                        2,
                        1,
                        Action::Update,
                        "取消保留",
                        false,
                    )?
                    .applied
                {
                    return Err("plain cancel setup rejected".into());
                }
                pump(Duration::from_millis(100));
                let abort = exchange(&pipe, &capability, &task, 2, 3, 1, Action::Abort, "", false)?;
                if abort.applied
                    || abort.mutation != Mutation::NotAttempted
                    || editor_text(editor)? != format!("{before}取消保留")
                {
                    return Err(format!("plain property-affected cancellation lost visible text: code={} applied={} mutation={:?} owned_fixture_expected={:?} owned_fixture_actual={:?}", abort.code, abort.applied, abort.mutation, format!("{before}取消保留"), editor_text(editor)?).into());
                }
                println!(
                    "owned plain EDIT cancellation retains property-affected text and ends composition=true"
                );
                return Ok(());
            }
            let original = "前 被选🙂 后";
            SendMessageW(
                editor,
                WM_SETTEXT,
                None,
                Some(LPARAM(
                    original
                        .encode_utf16()
                        .chain(Some(0))
                        .collect::<Vec<_>>()
                        .as_ptr() as isize,
                )),
            );
            SendMessageW(editor, 0xB1, Some(WPARAM(2)), Some(LPARAM(6))); // EM_SETSEL uses UTF16 offsets.
            pump(Duration::from_millis(50));
            for (operation, revision, action, text, expected) in [
                (5, 0, Action::Begin, "", original),
                (6, 1, Action::Update, "替换🙂", "前 替换🙂 后"),
                (7, 1, Action::Abort, "", original),
            ] {
                let reply = exchange(
                    &pipe,
                    &capability,
                    &task,
                    2,
                    operation,
                    revision,
                    action,
                    text,
                    false,
                )?;
                if !reply.applied || editor_text(editor)? != expected {
                    return Err("selection replacement/rollback assertion failed".into());
                }
            }
            println!("owned selection replacement and abort restore actual text=true");
            let before_expired = editor_text(editor)?;
            let expired = exchange(&pipe, &capability, &task, 3, 8, 0, Action::Begin, "", true)?;
            if expired.applied || editor_text(editor)? != before_expired {
                return Err("expired request modified target".into());
            }
            println!("owned expired request rejected without text mutation=true");
            exchange(&pipe, &capability, &task, 4, 9, 0, Action::Begin, "", false)?;
            let update = exchange(
                &pipe,
                &capability,
                &task,
                4,
                10,
                1,
                Action::Update,
                "交接文字",
                false,
            )?;
            if !update.applied {
                return Err("property case setup rejected".into());
            }
            let user_edit: ITfEditSession = UserPropertyEdit {
                context: context.clone(),
                mode: 0,
            }
            .into();
            context
                .RequestEditSession(0, &user_edit, TF_ES_SYNC | TF_ES_READWRITE)?
                .ok()?;
            pump(Duration::from_millis(100));
            verify_language(&context, 0x409)?;
            let corrected = exchange(
                &pipe,
                &capability,
                &task,
                4,
                11,
                2,
                Action::Update,
                "更新文字",
                false,
            )?;
            if !corrected.applied {
                return Err("property-preserving correction rejected".into());
            }
            verify_language(&context, 0x409)?;
            let handed_off_text = editor_text(editor)?;
            let abort = exchange(
                &pipe,
                &capability,
                &task,
                4,
                12,
                2,
                Action::Abort,
                "",
                false,
            )?;
            if abort.applied
                || abort.mutation != Mutation::NotAttempted
                || editor_text(editor)? != handed_off_text
            {
                return Err("external property edit was overwritten by rollback".into());
            }
            verify_language(&context, 0x409)?;
            println!("owned property-only user edit revoked rollback ownership=true");
            let handoff = exchange(
                &pipe,
                &capability,
                &task,
                4,
                13,
                2,
                Action::Handoff,
                "",
                false,
            )?;
            if !handoff.applied
                || handoff.code != "alreadyHandedOff"
                || handoff.mutation != Mutation::NotAttempted
                || editor_text(editor)? != handed_off_text
            {
                return Err("same-turn released range handoff acknowledgement changed text".into());
            }
            println!("owned released-range handoff acknowledged without mutation=true");
            // A property affecting only part of the owned text cannot be mapped
            // onto a replacement snapshot; deleting a property is likewise not
            // a scalar value that can be preserved.
            for (turn, mode) in [(6, 1), (7, 2)] {
                SendMessageW(
                    editor,
                    WM_SETTEXT,
                    None,
                    Some(LPARAM([0u16].as_ptr() as isize)),
                );
                SendMessageW(editor, 0xB1, Some(WPARAM(0)), Some(LPARAM(0)));
                pump(Duration::from_millis(50));
                for (operation, revision, action, text) in [
                    (1, 0, Action::Begin, ""),
                    (2, 1, Action::Update, "保留原文字"),
                ] {
                    if !exchange(
                        &pipe,
                        &capability,
                        &task,
                        turn,
                        operation,
                        revision,
                        action,
                        text,
                        false,
                    )?
                    .applied
                    {
                        return Err("attribute rejection setup failed".into());
                    }
                }
                let full: ITfEditSession = UserPropertyEdit {
                    context: context.clone(),
                    mode: 0,
                }
                .into();
                context
                    .RequestEditSession(0, &full, TF_ES_SYNC | TF_ES_READWRITE)?
                    .ok()?;
                let changed: ITfEditSession = UserPropertyEdit {
                    context: context.clone(),
                    mode,
                }
                .into();
                context
                    .RequestEditSession(0, &changed, TF_ES_SYNC | TF_ES_READWRITE)?
                    .ok()?;
                pump(Duration::from_millis(100));
                let preserved = editor_text(editor)?;
                let reply = exchange(
                    &pipe,
                    &capability,
                    &task,
                    turn,
                    3,
                    2,
                    Action::Update,
                    "不得覆盖",
                    false,
                )?;
                if reply.applied
                    || reply.mutation != Mutation::NotAttempted
                    || editor_text(editor)? != preserved
                {
                    return Err("partial/cleared attribute allowed later overwrite".into());
                }
                println!(
                    "owned attribute mode={mode} hands off and rejects stale replacement=true"
                );
            }
            SendMessageW(
                editor,
                WM_SETTEXT,
                None,
                Some(LPARAM([0u16].as_ptr() as isize)),
            );
            SendMessageW(editor, 0xB1, Some(WPARAM(0)), Some(LPARAM(0)));
            pump(Duration::from_millis(50));
            let multiline = "中文，\n换行🙂";
            for (operation, revision, action, text) in [
                (13, 0, Action::Begin, ""),
                (14, 1, Action::Update, multiline),
                (15, 1, Action::Commit, ""),
            ] {
                let reply = exchange(
                    &pipe,
                    &capability,
                    &task,
                    8,
                    operation,
                    revision,
                    action,
                    text,
                    false,
                )?;
                if !reply.applied {
                    return Err(format!("multiline edit rejected: {}", reply.code).into());
                }
                if action != Action::Begin
                    && editor_text(editor)?
                        .replace("\r\n", "\n")
                        .replace('\r', "\n")
                        != multiline
                {
                    return Err("multiline actual target text incorrect".into());
                }
            }
            println!("owned multiline and emoji update/commit actual text=true");
            // Exercise TSF's actual key dispatcher, not a direct call to our sink.
            // It must hand off before the application edits, without eating the key.
            for (op, revision, action, text) in [
                (16, 0, Action::Begin, ""),
                (17, 1, Action::Update, "删除字"),
            ] {
                if !exchange(
                    &pipe,
                    &capability,
                    &task,
                    9,
                    op,
                    revision,
                    action,
                    text,
                    false,
                )?
                .applied
                {
                    return Err("keyboard handoff setup rejected".into());
                }
            }
            let keys = manager.cast::<ITfKeystrokeMgr>()?;
            if !keys
                .TestKeyDown(WPARAM(VK_BACK.0 as usize), LPARAM(1))?
                .as_bool()
            {
                return Err("owned composition did not receive Backspace test".into());
            }
            if keys
                .KeyDown(WPARAM(VK_BACK.0 as usize), LPARAM(1))?
                .as_bool()
            {
                return Err("composition handoff swallowed Backspace".into());
            }
            SendMessageW(
                editor,
                WM_KEYDOWN,
                Some(WPARAM(VK_BACK.0 as usize)),
                Some(LPARAM(1)),
            );
            pump(Duration::from_millis(100));
            let handoff = exchange(
                &pipe,
                &capability,
                &task,
                9,
                18,
                1,
                Action::Handoff,
                "",
                false,
            )?;
            if !handoff.applied
                || handoff.code != "keyboardHandedOff"
                || !editor_text(editor)?.ends_with("删除")
            {
                return Err(format!("keyboard handoff restored deleted text or missed pre-edit dispatch: code={} applied={} fixture_text={:?}", handoff.code, handoff.applied, editor_text(editor)?).into());
            }
            println!("owned pre-edit Backspace handoff preserves actual deletion=true");
            for semantic in [
                "\n开头🙂",
                "结尾\n",
                "前\n\n后",
                "前\r\n中\r后",
                "前🙂\n组合 e\u{301}",
            ] {
                let original = "前 被选🙂 后";
                SendMessageW(
                    editor,
                    WM_SETTEXT,
                    None,
                    Some(LPARAM(
                        original
                            .encode_utf16()
                            .chain(Some(0))
                            .collect::<Vec<_>>()
                            .as_ptr() as isize,
                    )),
                );
                SendMessageW(editor, 0xB1, Some(WPARAM(2)), Some(LPARAM(6)));
                pump(Duration::from_millis(30));
                let sample_task = uuid::Uuid::new_v4().to_string();
                for (operation, revision, action, text) in [
                    (1, 0, Action::Begin, ""),
                    (2, 1, Action::Update, "临时\n片段"),
                    (3, 2, Action::Update, semantic),
                    (4, 2, Action::Commit, ""),
                ] {
                    let reply = exchange(
                        &pipe,
                        &capability,
                        &sample_task,
                        1,
                        operation,
                        revision,
                        action,
                        text,
                        false,
                    )?;
                    if !reply.applied {
                        return Err(
                            format!("multiline boundary case rejected: {}", reply.code).into()
                        );
                    }
                }
                let actual = editor_text(editor)?
                    .replace("\r\n", "\n")
                    .replace('\r', "\n");
                let expected = format!(
                    "前 {} 后",
                    semantic.replace("\r\n", "\n").replace('\r', "\n")
                );
                if actual != expected {
                    return Err(
                        "multiline boundary commit changed original surroundings or text".into(),
                    );
                }
            }
            println!(
                "owned leading/trailing/repeated/mixed breaks, revised snapshots and Unicode exact text=true"
            );
            Ok(())
        })();
        if let Err(error) = &result {
            eprintln!("owned fixture assertion: {error}");
        }
        drop(host);
        let stopped = if installed {
            registration::deactivate(language, false)
        } else {
            processor
                .as_ref()
                .ok_or("missing direct fixture processor")?
                .Deactivate()
        };
        drop(processor);
        let manager_stopped = manager.Deactivate();
        let _ = DestroyWindow(if external_parent.is_some() {
            editor
        } else {
            top
        });
        let _ = FreeLibrary(library);
        pump(Duration::from_millis(100));
        let entry = GetProcAddress(component, windows::core::s!("DllCanUnloadNow"))
            .ok_or("missing unload export")?;
        let can_unload: unsafe extern "system" fn() -> windows::core::HRESULT =
            std::mem::transmute(entry);
        if can_unload() != windows::Win32::Foundation::S_OK {
            return Err("component still owns resources after fixture cleanup".into());
        }
        FreeLibrary(component)?;
        println!("owned COM DLL unloaded with no live leases=true");
        result?;
        stopped.map_err(|e| format!("owned service deactivation: {e}"))?;
        manager_stopped.map_err(|e| format!("owned manager deactivation: {e}"))?;
        if installed && keyboard_before != Some(keyboard()?) {
            return Err("owned process TIP cleanup changed keyboard identity".into());
        }
        Ok(())
    }
}
#[cfg(feature = "acceptance")]
fn composition_text(context: &ITfContext) -> Result<String, Box<dyn std::error::Error>> {
    let output = std::rc::Rc::new(std::cell::RefCell::new(None));
    let read: ITfEditSession = ReadComposition {
        context: context.clone(),
        output: output.clone(),
    }
    .into();
    unsafe {
        context
            .RequestEditSession(0, &read, TF_ES_ASYNCDONTCARE | TF_ES_READ)?
            .ok()?;
    }
    let started = Instant::now();
    loop {
        if let Some(value) = output.borrow_mut().take() {
            return Ok(value?);
        }
        if started.elapsed() > Duration::from_secs(2) {
            return Err("owned composition read timeout".into());
        }
        pump(Duration::from_millis(10));
    }
}
#[cfg(feature = "acceptance")]
#[windows::core::implement(ITfEditSession)]
struct ReadComposition {
    context: ITfContext,
    output: std::rc::Rc<std::cell::RefCell<Option<windows::core::Result<String>>>>,
}
#[cfg(feature = "acceptance")]
impl ITfEditSession_Impl for ReadComposition_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        let result = (|| unsafe {
            let compositions: ITfContextComposition = self.context.cast()?;
            let mut composition = [None];
            let mut count = 0;
            compositions
                .EnumCompositions()?
                .Next(&mut composition, &mut count)?;
            if count != 1 {
                return Err(windows::core::Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    "owned composition missing",
                ));
            }
            let range = composition[0].as_ref().unwrap().GetRange()?;
            let mut text = [0u16; 256];
            let mut fetched = 0;
            range.GetText(ec, 0, &mut text, &mut fetched)?;
            String::from_utf16(&text[..fetched as usize])
                .map_err(|_| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))
        })();
        *self.output.borrow_mut() = Some(result);
        Ok(())
    }
}
#[cfg(feature = "acceptance")]
fn load_fixture_module(
    path: &std::path::Path,
) -> Result<windows::Win32::Foundation::HMODULE, Box<dyn std::error::Error>> {
    use windows::{Win32::System::LibraryLoader::*, core::PCWSTR};
    let path: Vec<_> = std::fs::canonicalize(path)?
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    Ok(unsafe {
        LoadLibraryExW(
            PCWSTR(path.as_ptr()),
            None,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )?
    })
}
#[cfg(feature = "acceptance")]
fn load_processor(
    path: &std::path::Path,
) -> Result<(windows::Win32::Foundation::HMODULE, ITfTextInputProcessor), Box<dyn std::error::Error>>
{
    use windows::{
        Win32::{Foundation::FreeLibrary, System::LibraryLoader::*},
        core::{GUID, HRESULT, Interface, s},
    };
    unsafe {
        let module = load_fixture_module(path)?;
        let result = (|| -> Result<ITfTextInputProcessor, Box<dyn std::error::Error>> {
            let entry = GetProcAddress(module, s!("DllGetClassObject"))
                .ok_or("missing COM factory export")?;
            let factory_entry: unsafe extern "system" fn(
                *const GUID,
                *const GUID,
                *mut *mut std::ffi::c_void,
            ) -> HRESULT = std::mem::transmute(entry);
            let mut raw = std::ptr::null_mut();
            factory_entry(&registration::CLASS, &IClassFactory::IID, &mut raw).ok()?;
            let factory = IClassFactory::from_raw(raw);
            Ok(factory.CreateInstance(None)?)
        })();
        match result {
            Ok(processor) => Ok((module, processor)),
            Err(e) => {
                let _ = FreeLibrary(module);
                Err(e)
            }
        }
    }
}
#[cfg(feature = "acceptance")]
#[windows::core::implement(ITfEditSession)]
struct UserPropertyEdit {
    context: ITfContext,
    mode: u8,
}
#[cfg(feature = "acceptance")]
#[windows::core::implement(ITfEditSession)]
struct UserTextEdit {
    context: ITfContext,
    start: i32,
    length: i32,
    text: String,
}
#[cfg(feature = "acceptance")]
fn observation_ranges_fixture(
    target: (
        windows::Win32::Foundation::HMODULE,
        windows::Win32::Foundation::HWND,
    ),
    context: &ITfContext,
    manager: &ITfThreadMgr,
    pipe: &Pipe,
    capability: &str,
    foreign: u32,
    plain: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (component, editor) = target;
    use windows::{
        Win32::{Foundation::*, UI::Input::KeyboardAndMouse::*},
        core::PCWSTR,
    };
    unsafe {
        for (label, input, from, to, replacement, expected, valid) in [
            (
                "longer",
                "术语🙂\n尾部",
                0usize,
                2usize,
                "正确术语",
                "正确术语🙂\n尾部",
                true,
            ),
            (
                "shorter",
                "正确术语🙂\n尾部",
                0,
                4,
                "词",
                "词🙂\n尾部",
                true,
            ),
            (
                "surrogate",
                "术语🙂\n尾部",
                2,
                4,
                "😀",
                "术语😀\n尾部",
                true,
            ),
            ("outside", "同词", 0, 2, "新词", "新词", false),
            ("right", "同词", 2, 2, "新词", "同词新词", false),
            ("ime", "术语", 0, 2, "词典", "词典", true),
            ("undo", "术语", 0, 2, "词典", "词典", true),
            ("revert", "术语", 0, 2, "词典", "词典", true),
            ("handoffExplicit", "术语", 0, 2, "词典", "词典", true),
            ("clearAfterEdit", "术语", 0, 2, "词典", "词典", true),
            ("settleClearAfterEdit", "术语", 0, 2, "词典", "词典", true),
            ("clearWithoutEdit", "术语", 0, 2, "术语", "术语", false),
            ("clearRestored", "术语", 0, 2, "词典", "词典", false),
            ("rangeClear", "术语", 0, 2, "", "", true),
            ("rangeClearAfterEdit", "术语", 0, 2, "词典", "词典", true),
            ("clearComposing", "术语", 0, 2, "词典", "词典", false),
            ("clearComposingEnded", "术语", 0, 2, "词典", "词典", false),
            ("clearIncomplete", "术语", 0, 2, "词典", "词典", false),
            ("readonly", "术语", 0, 2, "词典", "词典", true),
            ("password", "术语", 0, 2, "词典", "词典", true),
            ("originalRestore", "术语", 0, 2, "甲词", "甲词", false),
            ("settleHandoff", "术语", 0, 2, "词典", "词典", true),
            // Read-only can replace the host context. Releasing an already
            // submitted lease must succeed, but must not reclaim observation.
            ("settleReadonly", "术语", 0, 2, "词典", "词典", false),
            ("settleRebase", "术语", 0, 2, "词典", "词典", true),
            ("settleReentry", "术语", 0, 2, "词典", "词典", true),
            ("settleReactivate", "术语", 0, 2, "词典", "词典", false),
        ] {
            if !plain
                && matches!(
                    label,
                    "ime" | "clearComposing" | "clearIncomplete" | "undo" | "readonly" | "password"
                )
            {
                continue;
            }
            // Legacy EDIT may expose an empty TSF original even with a native
            // selection. Its real WM_UNDO is tested above; do not invent an
            // original-text proof for an ordinary typed replacement.
            if plain && label == "originalRestore" {
                continue;
            }
            if plain && label.starts_with("settle") {
                continue;
            }
            if plain && label == "clearComposingEnded" {
                continue;
            }
            let user_composition = std::rc::Rc::new(std::cell::RefCell::new(None));
            let prefix = if label == "outside" {
                "同词"
            } else if matches!(label, "rangeClear" | "rangeClearAfterEdit") {
                "正文"
            } else {
                ""
            };
            let (from, to) = if matches!(label, "rangeClear" | "rangeClearAfterEdit") {
                (
                    from + prefix.encode_utf16().count(),
                    to + prefix.encode_utf16().count(),
                )
            } else {
                (from, to)
            };
            let initial_body = if label == "originalRestore" {
                "甲词"
            } else {
                prefix
            };
            let initial = initial_body.encode_utf16().chain([0]).collect::<Vec<_>>();
            SetWindowTextW(editor, PCWSTR(initial.as_ptr()))?;
            SendMessageW(
                editor,
                0x00B1,
                Some(WPARAM(prefix.encode_utf16().count())),
                Some(LPARAM(if label == "originalRestore" {
                    initial_body.encode_utf16().count() as isize
                } else {
                    prefix.encode_utf16().count() as isize
                })),
            );
            pump(Duration::from_millis(60));
            let task = uuid::Uuid::new_v4().to_string();
            let mut range_epoch = 0;
            for (op, rev, action, text) in [
                (1, 0, Action::Begin, ""),
                (2, 0, Action::ObserveStart, ""),
                (3, 1, Action::Update, input),
                (4, 1, Action::Commit, ""),
            ] {
                let action = if label == "handoffExplicit" && action == Action::Commit {
                    Action::Handoff
                } else {
                    action
                };
                let result = if label.starts_with("settle") && action == Action::Commit {
                    use windows::Win32::System::LibraryLoader::GetProcAddress;
                    let entry =
                        GetProcAddress(component, windows::core::s!("SayweaveSettleFixtureFault"))
                            .ok_or("settle fault hook missing")?;
                    let inject: unsafe extern "system" fn(usize) = std::mem::transmute(entry);
                    inject(6);
                    let request = Request {
                        revoke: None,
                        block: None,
                        engine: fixture_binding(),
                        protocol: PROTOCOL,
                        capability: capability.into(),
                        task: task.clone(),
                        turn: 1,
                        operation: op,
                        revision: 2,
                        deadline_tick: windows::Win32::System::SystemInformation::GetTickCount64()
                            + 2000,
                        action: Action::Settle,
                        text: String::new(),
                        prefix: Some(Prefix {
                            text: input.into(),
                            range_epoch,
                            expected_revision: 1,
                        }),
                    };
                    pipe.send(&serde_json::to_vec(&request)?)?;
                    let started = Instant::now();
                    loop {
                        pump(Duration::from_millis(10));
                        if let Some(frame) = pipe.receive()? {
                            break serde_json::from_slice::<Reply>(&frame)?;
                        }
                        if started.elapsed() > Duration::from_secs(2) {
                            return Err("owned settlement receipt timeout".into());
                        }
                    }
                } else {
                    exchange(pipe, capability, &task, 1, op, rev, action, text, false)?
                };
                if !result.applied {
                    return Err(format!("receipt setup {label}: {}", result.code).into());
                }
                range_epoch = result.range_epoch;
            }
            if label == "settleReentry" {
                let update = exchange(
                    pipe,
                    capability,
                    &task,
                    1,
                    5,
                    3,
                    Action::Update,
                    input,
                    false,
                )?;
                let request = Request {
                    revoke: None,
                    block: None,
                    engine: fixture_binding(),
                    protocol: PROTOCOL,
                    capability: capability.into(),
                    task: task.clone(),
                    turn: 1,
                    operation: 6,
                    revision: 4,
                    deadline_tick: windows::Win32::System::SystemInformation::GetTickCount64()
                        + 2000,
                    action: Action::Settle,
                    text: String::new(),
                    prefix: Some(Prefix {
                        text: input.into(),
                        range_epoch: update.range_epoch,
                        expected_revision: 3,
                    }),
                };
                pipe.send(&serde_json::to_vec(&request)?)?;
                let started = Instant::now();
                loop {
                    pump(Duration::from_millis(10));
                    if let Some(frame) = pipe.receive()? {
                        let reply: Reply = serde_json::from_slice(&frame)?;
                        if !reply.applied {
                            return Err(format!("second observed prefix: {}", reply.code).into());
                        }
                        break;
                    }
                    if started.elapsed() > Duration::from_secs(2) {
                        return Err("second observed prefix timeout".into());
                    }
                }
            }
            if matches!(label, "settleHandoff" | "settleReadonly") {
                if label == "settleReadonly" {
                    SendMessageW(editor, 0x00CF, Some(WPARAM(1)), None);
                }
                let reply = exchange(pipe, capability, &task, 1, 5, 2, Action::Handoff, "", false)?;
                if !reply.applied || reply.mutation != Mutation::NotAttempted {
                    return Err(format!("settle handoff: {}", reply.code).into());
                }
                if !observation_text_equal(&editor_text(editor)?, input) {
                    return Err("settled lease release changed actual text".into());
                }
                if label == "settleReadonly" {
                    SendMessageW(editor, 0x00CF, Some(WPARAM(0)), None);
                }
            }
            pump(Duration::from_millis(120));
            let before = if label == "handoff" && plain {
                composition_text(context)?
            } else {
                editor_text(editor)?
            };
            let before_expected = if label == "settleReentry" {
                format!("{input}{input}")
            } else {
                format!("{prefix}{input}")
            };
            if !observation_text_equal(&before, &before_expected) {
                return Err(format!("receipt initial text {label}").into());
            }
            if label == "undo" {
                SendMessageW(editor, 0x00CD, None, None);
            }
            if label == "handoff" {
                let dispatch = manager.cast::<ITfKeystrokeMgr>()?;
                if !dispatch
                    .TestKeyDown(WPARAM(VK_BACK.0 as usize), LPARAM(1))?
                    .as_bool()
                    || dispatch
                        .KeyDown(WPARAM(VK_BACK.0 as usize), LPARAM(1))?
                        .as_bool()
                {
                    return Err("handoff dispatcher did not forward original key".into());
                }
                let keys = [
                    (VK_CONTROL, false),
                    (VIRTUAL_KEY(0x41), false),
                    (VIRTUAL_KEY(0x41), true),
                    (VK_CONTROL, true),
                ]
                .map(|(key, up)| INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: key,
                            dwFlags: if up {
                                KEYEVENTF_KEYUP
                            } else {
                                KEYBD_EVENT_FLAGS(0)
                            },
                            ..Default::default()
                        },
                    },
                });
                if SendInput(&keys, std::mem::size_of::<INPUT>() as i32) != keys.len() as u32 {
                    return Err("handoff keys incomplete".into());
                }
                pump(Duration::from_millis(60));
                if editor_text(editor)? != input {
                    return Err("handoff actual text was not committed before the key".into());
                }
            }
            // Native EDIT uses CRLF coordinates. RichEdit exposes one CR per line.
            // These changes stay before the first newline, so coordinates are exact.
            if plain {
                if matches!(label, "ime" | "clearComposing") {
                    SendMessageW(editor, WM_IME_STARTCOMPOSITION, None, None);
                }
                SendMessageW(
                    editor,
                    0x00B1,
                    Some(WPARAM(from)),
                    Some(LPARAM(to as isize)),
                );
                if replacement.is_empty() {
                    SendMessageW(editor, WM_CHAR, Some(WPARAM(8)), None);
                }
                let keys = replacement
                    .encode_utf16()
                    .flat_map(|unit| {
                        [false, true].map(move |up| INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wScan: unit,
                                    dwFlags: KEYEVENTF_UNICODE
                                        | if up {
                                            KEYEVENTF_KEYUP
                                        } else {
                                            KEYBD_EVENT_FLAGS(0)
                                        },
                                    ..Default::default()
                                },
                            },
                        })
                    })
                    .collect::<Vec<_>>();
                if label == "undo" {
                    let text = replacement.encode_utf16().chain([0]).collect::<Vec<_>>();
                    SendMessageW(
                        editor,
                        0x00C2,
                        Some(WPARAM(1)),
                        Some(LPARAM(text.as_ptr() as isize)),
                    );
                } else if SendInput(&keys, std::mem::size_of::<INPUT>() as i32) != keys.len() as u32
                {
                    return Err("receipt input incomplete".into());
                }
            } else {
                if matches!(label, "settleReentry" | "settleReactivate") {
                    use windows::Win32::System::LibraryLoader::GetProcAddress;
                    let entry =
                        GetProcAddress(component, windows::core::s!("SayweaveSettleFixtureFault"))
                            .ok_or("observation reentry hook missing")?;
                    let inject: unsafe extern "system" fn(usize) = std::mem::transmute(entry);
                    inject(if label == "settleReactivate" { 10 } else { 8 });
                }
                let edit: ITfEditSession = if label == "clearComposingEnded" {
                    ComposingChange {
                        context: context.clone(),
                        composition: user_composition.clone(),
                        start: true,
                    }
                    .into()
                } else {
                    UserTextEdit {
                        context: context.clone(),
                        start: from as i32,
                        length: (to - from) as i32,
                        text: replacement.into(),
                    }
                    .into()
                };
                context
                    .RequestEditSession(foreign, &edit, TF_ES_SYNC | TF_ES_READWRITE)?
                    .ok()?;
            }
            pump(Duration::from_millis(50));
            let actual = editor_text(editor)?;
            let expected_all = if label == "settleReentry" {
                format!("{expected}{input}")
            } else if matches!(label, "rangeClear" | "rangeClearAfterEdit") {
                format!("{prefix}{expected}")
            } else if label != "outside" {
                expected.to_owned()
            } else {
                format!("{expected}{input}")
            };
            if !observation_text_equal(&actual, &expected_all) {
                return Err(format!("receipt actual text {label}: {actual:?}").into());
            }
            if label == "settleReactivate" {
                use windows::Win32::System::LibraryLoader::GetProcAddress;
                let entry = GetProcAddress(
                    component,
                    windows::core::s!("SayweaveObservationFixtureRetired"),
                )
                .ok_or("observation retirement hook missing")?;
                let retired: unsafe extern "system" fn() -> bool = std::mem::transmute(entry);
                if !retired() {
                    return Err(
                        "reentrant retirement did not drain or retired the new activation".into(),
                    );
                }
                println!(
                    "owned receipt scenario=settleReactivate actual_text=true old_resources_retired=true new_activation_retained=true"
                );
                continue;
            }
            if matches!(
                label,
                "clearAfterEdit" | "settleClearAfterEdit" | "clearWithoutEdit" | "clearComposing"
            ) {
                // Simulate a host submitting and clearing its field immediately
                // after a verified correction, before the first observation poll.
                SetWindowTextW(editor, windows::core::w!(""))?;
                pump(Duration::from_millis(30));
                if !editor_text(editor)?.is_empty() {
                    return Err("submitted fixture field was not cleared".into());
                }
            }
            if label == "clearComposingEnded" {
                let interim = exchange(
                    pipe,
                    capability,
                    &task,
                    1,
                    5,
                    1,
                    Action::ObservePoll,
                    "",
                    false,
                )?
                .edit_observation
                .ok_or("missing composing snapshot")?;
                if interim.active == 0 || !interim.records.is_empty() {
                    return Err(
                        "active uncommitted composition was exposed or observation lost".into(),
                    );
                }
                let edit: ITfEditSession = ComposingChange {
                    context: context.clone(),
                    composition: user_composition.clone(),
                    start: false,
                }
                .into();
                context
                    .RequestEditSession(foreign, &edit, TF_ES_SYNC | TF_ES_READWRITE)?
                    .ok()?;
                pump(Duration::from_millis(30));
                if !editor_text(editor)?.is_empty() {
                    return Err("IME end/clear did not empty actual field".into());
                }
            }
            if label == "clearIncomplete" {
                let size = expected.encode_utf16().count();
                SendMessageW(
                    editor,
                    0x00B1,
                    Some(WPARAM(size)),
                    Some(LPARAM(size as isize)),
                );
                SendMessageW(editor, WM_CHAR, Some(WPARAM(0xd83d)), None);
                if SendMessageW(editor, WM_GETTEXTLENGTH, None, None).0 != (size + 1) as isize {
                    return Err("incomplete surrogate fixture was not inserted".into());
                }
                SendMessageW(
                    editor,
                    0x00B1,
                    Some(WPARAM(0)),
                    Some(LPARAM((size + 1) as isize)),
                );
                SendMessageW(editor, WM_CHAR, Some(WPARAM(8)), None);
                if !editor_text(editor)?.is_empty() {
                    return Err("incomplete fixture was not cleared".into());
                }
            }
            if label == "rangeClearAfterEdit" {
                if plain {
                    SendMessageW(
                        editor,
                        0x00B1,
                        Some(WPARAM(from)),
                        Some(LPARAM(to as isize)),
                    );
                    SendMessageW(editor, WM_CHAR, Some(WPARAM(8)), None);
                } else {
                    let edit: ITfEditSession = UserTextEdit {
                        context: context.clone(),
                        start: from as i32,
                        length: (to - from) as i32,
                        text: String::new(),
                    }
                    .into();
                    context
                        .RequestEditSession(foreign, &edit, TF_ES_SYNC | TF_ES_READWRITE)?
                        .ok()?;
                }
                pump(Duration::from_millis(30));
                if editor_text(editor)? != prefix {
                    return Err("cleared receipt changed neighboring text".into());
                }
            }
            if label == "ime" {
                let reading = exchange(
                    pipe,
                    capability,
                    &task,
                    1,
                    5,
                    1,
                    Action::ObservePoll,
                    "",
                    false,
                )?;
                if !reading
                    .edit_observation
                    .ok_or("missing IME receipt")?
                    .records
                    .is_empty()
                {
                    return Err("incomplete IME was exposed".into());
                }
                SendMessageW(editor, WM_IME_ENDCOMPOSITION, None, None);
            }
            if label == "undo" {
                let keys = [
                    (VK_CONTROL, false),
                    (
                        windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(0x5a),
                        false,
                    ),
                    (
                        windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(0x5a),
                        true,
                    ),
                    (VK_CONTROL, true),
                ]
                .map(|(key, up)| INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: key,
                            dwFlags: if up {
                                KEYEVENTF_KEYUP
                            } else {
                                KEYBD_EVENT_FLAGS(0)
                            },
                            ..Default::default()
                        },
                    },
                });
                if SendInput(&keys, std::mem::size_of::<INPUT>() as i32) != keys.len() as u32 {
                    return Err("undo keys incomplete".into());
                }
                pump(Duration::from_millis(30));
                // Legacy EDIT keeps one undo group for the preceding IMM delivery;
                // this actual Ctrl+Z clears that group. Observation must not learn it.
                if !editor_text(editor)?.is_empty() {
                    return Err(format!(
                        "undo target did not restore original text: fixture_actual={:?}",
                        editor_text(editor)?
                    )
                    .into());
                }
            }
            if matches!(label, "revert" | "clearRestored") {
                if plain {
                    SendMessageW(
                        editor,
                        0x00B1,
                        Some(WPARAM(0)),
                        Some(LPARAM(expected.encode_utf16().count() as isize)),
                    );
                    let text = input.encode_utf16().chain([0]).collect::<Vec<_>>();
                    SendMessageW(
                        editor,
                        0x00C2,
                        Some(WPARAM(1)),
                        Some(LPARAM(text.as_ptr() as isize)),
                    );
                } else {
                    let edit: ITfEditSession = UserTextEdit {
                        context: context.clone(),
                        start: 0,
                        length: expected.encode_utf16().count() as i32,
                        text: input.into(),
                    }
                    .into();
                    context
                        .RequestEditSession(foreign, &edit, TF_ES_SYNC | TF_ES_READWRITE)?
                        .ok()?;
                }
                pump(Duration::from_millis(30));
                if !observation_text_equal(&editor_text(editor)?, input) {
                    return Err("revert did not restore actual output".into());
                }
                if label == "clearRestored" {
                    SetWindowTextW(editor, windows::core::w!(""))?;
                    pump(Duration::from_millis(30));
                    if !editor_text(editor)?.is_empty() {
                        return Err("restored fixture was not cleared".into());
                    }
                }
            }
            let read_count = |operation| -> Result<u64, Box<dyn std::error::Error>> {
                let reply = exchange(
                    pipe,
                    capability,
                    &task,
                    1,
                    operation,
                    1,
                    Action::Inspect,
                    "",
                    false,
                )?;
                Ok(reply
                    .code
                    .split_whitespace()
                    .find_map(|v| v.strip_prefix("observationReads="))
                    .ok_or("acceptance read counter missing")?
                    .parse()?)
            };
            let guarded = matches!(label, "readonly" | "password");
            let before_reads = if guarded { Some(read_count(8)?) } else { None };
            let original_style = GetWindowLongPtrW(editor, GWL_STYLE);
            if label == "readonly" {
                SendMessageW(editor, 0x00CF, Some(WPARAM(1)), None);
            }
            if label == "password" {
                SetWindowLongPtrW(editor, GWL_STYLE, original_style | ES_PASSWORD as isize);
            }
            if guarded {
                let end = expected.encode_utf16().count();
                SendMessageW(
                    editor,
                    0x00B1,
                    Some(WPARAM(end)),
                    Some(windows::Win32::Foundation::LPARAM(end as isize)),
                );
                SendMessageW(editor, WM_CHAR, Some(WPARAM(b'X' as usize)), None);
                pump(Duration::from_millis(30));
            }
            let result = exchange(
                pipe,
                capability,
                &task,
                1,
                6,
                1,
                Action::ObserveStop,
                "",
                false,
            )?;
            let records = result
                .edit_observation
                .ok_or("receipt response missing")?
                .records;
            if let Some(before_reads) = before_reads {
                if read_count(9)? != before_reads {
                    return Err("protected/readonly observation performed a new span read".into());
                }
                let expected_actual = if label == "password" {
                    format!("{expected}X")
                } else {
                    expected.into()
                };
                if !observation_text_equal(&editor_text(editor)?, &expected_actual) {
                    return Err("observation retirement changed native edit behavior".into());
                }
                println!(
                    "owned receipt scenario={label} native_observation_reads=0 original_message_delivered=true"
                );
            }
            if label == "readonly" {
                SendMessageW(editor, 0x00CF, Some(WPARAM(0)), None);
            }
            if label == "password" {
                SetWindowLongPtrW(editor, GWL_STYLE, original_style);
            }
            let valid = valid && !matches!(label, "undo" | "readonly" | "password" | "revert");
            let cleared_negative = matches!(
                label,
                "clearWithoutEdit" | "clearRestored" | "clearComposingEnded" | "clearIncomplete"
            );
            if result.mutation != Mutation::NotAttempted
                || (valid
                    && (records.len() != 1
                        || !observation_text_equal(&records[0].before, input)
                        || !observation_text_equal(&records[0].after, expected)))
                || (!valid && !cleared_negative && !records.is_empty())
                || (cleared_negative && records.iter().any(|record| !record.after.is_empty()))
            {
                return Err(format!(
                    "receipt scope assertion {label} records={} diagnostic={} fixture_values={:?}",
                    records.len(),
                    result.code,
                    records
                        .iter()
                        .map(|r| (&r.before, &r.after, r.composing, r.ended))
                        .collect::<Vec<_>>()
                )
                .into());
            }
            println!("owned receipt scenario={label} actual_text=true scoped_edit=true");
        }
        Ok(())
    }
}
#[cfg(feature = "acceptance")]
fn observation_text_equal(actual: &str, expected: &str) -> bool {
    actual.replace("\r\n", "\n").replace('\r', "\n")
        == expected.replace("\r\n", "\n").replace('\r', "\n")
}
#[cfg(feature = "acceptance")]
impl ITfEditSession_Impl for UserTextEdit_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        unsafe {
            let range = self.context.GetStart(ec)?;
            let mut shifted = 0;
            range.ShiftStart(ec, self.start, &mut shifted, std::ptr::null())?;
            if shifted != self.start {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_FAIL,
                ));
            }
            range.ShiftEnd(ec, self.length, &mut shifted, std::ptr::null())?;
            if shifted != self.length {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_FAIL,
                ));
            }
            range.SetText(
                ec,
                TF_ST_CORRECTION,
                &self.text.encode_utf16().collect::<Vec<_>>(),
            )
        }
    }
}
#[cfg(feature = "acceptance")]
#[windows::core::implement(ITfCompositionSink)]
struct UserCompositionSink;
#[cfg(feature = "acceptance")]
impl ITfCompositionSink_Impl for UserCompositionSink_Impl {
    fn OnCompositionTerminated(
        &self,
        _: u32,
        _: windows::core::Ref<ITfComposition>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}
#[cfg(feature = "acceptance")]
#[windows::core::implement(ITfEditSession)]
struct ComposingChange {
    context: ITfContext,
    composition: std::rc::Rc<std::cell::RefCell<Option<ITfComposition>>>,
    start: bool,
}
#[cfg(feature = "acceptance")]
impl ITfEditSession_Impl for ComposingChange_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        unsafe {
            let range = self.context.GetStart(ec)?;
            if self.start {
                let mut moved = 0;
                range.ShiftEnd(ec, 2, &mut moved, std::ptr::null())?;
                if moved != 2 {
                    return Err(windows::core::Error::from_hresult(
                        windows::Win32::Foundation::E_FAIL,
                    ));
                }
                let sink: ITfCompositionSink = UserCompositionSink.into();
                let owner: ITfContextComposition = self.context.cast()?;
                *self.composition.borrow_mut() = Some(owner.StartComposition(ec, &range, &sink)?);
                range.SetText(
                    ec,
                    TF_ST_CORRECTION,
                    &"词典".encode_utf16().collect::<Vec<_>>(),
                )
            } else {
                let composition = self.composition.borrow_mut().take().ok_or_else(|| {
                    windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL)
                })?;
                composition.EndComposition(ec)?;
                let owner: ITfContextComposition = self.context.cast()?;
                let mut compositions = [None];
                let mut count = 0;
                owner
                    .EnumCompositions()?
                    .Next(&mut compositions, &mut count)?;
                if count != 0 {
                    return Err(windows::core::Error::from_hresult(
                        windows::Win32::Foundation::E_FAIL,
                    ));
                }
                // Clear through the actual host in the same foreign edit
                // session. Range SetText can leave RichEdit's terminal CR.
                SetWindowTextW(
                    self.context.GetActiveView()?.GetWnd()?,
                    windows::core::w!(""),
                )
            }
        }
    }
}
#[cfg(feature = "acceptance")]
#[windows::core::implement(ITfEditSession)]
struct ReadLanguage {
    context: ITfContext,
    result: std::rc::Rc<std::cell::Cell<Option<i32>>>,
}
#[cfg(feature = "acceptance")]
impl ITfEditSession_Impl for ReadLanguage_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        unsafe {
            let range = self.context.GetStart(ec)?;
            range.ShiftEndToRange(ec, &self.context.GetEnd(ec)?, TF_ANCHOR_END)?;
            let value = self
                .context
                .GetProperty(&GUID_PROP_LANGID)?
                .GetValue(ec, &range)?;
            eprintln!(
                "language proof type={} value={:?}",
                value.vt().0,
                i32::try_from(&value).ok()
            );
            if value.vt() != windows::Win32::System::Variant::VT_I4 {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_FAIL,
                ));
            }
            self.result.set(Some(i32::try_from(&value)?));
            Ok(())
        }
    }
}
#[cfg(feature = "acceptance")]
fn verify_language(context: &ITfContext, expected: i32) -> Result<(), Box<dyn std::error::Error>> {
    let result = std::rc::Rc::new(std::cell::Cell::new(None));
    let read: ITfEditSession = ReadLanguage {
        context: context.clone(),
        result: result.clone(),
    }
    .into();
    unsafe {
        context
            .RequestEditSession(0, &read, TF_ES_SYNC | TF_ES_READ)?
            .ok()?;
    }
    if result.get() != Some(expected) {
        return Err("user LANGID changed during correction/rollback".into());
    }
    Ok(())
}
#[cfg(feature = "acceptance")]
impl ITfEditSession_Impl for UserPropertyEdit_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        unsafe {
            let range = self.context.GetStart(ec)?;
            range.ShiftEndToRange(ec, &self.context.GetEnd(ec)?, TF_ANCHOR_END)?;
            if self.mode == 1 {
                range.Collapse(ec, TF_ANCHOR_START)?;
                let mut shifted = 0;
                range.ShiftEnd(ec, 1, &mut shifted, std::ptr::null())?;
            }
            if self.mode == 2 {
                return self
                    .context
                    .GetProperty(&GUID_PROP_LANGID)?
                    .Clear(ec, &range);
            }
            self.context.GetProperty(&GUID_PROP_LANGID)?.SetValue(
                ec,
                &range,
                &windows::Win32::System::Variant::VARIANT::from(if self.mode == 1 {
                    0x804i32
                } else {
                    0x409i32
                }),
            )
        }
    }
}
#[cfg(feature = "acceptance")]
fn editor_text(
    editor: windows::Win32::Foundation::HWND,
) -> Result<String, Box<dyn std::error::Error>> {
    use windows::Win32::Foundation::*;
    unsafe {
        let length = SendMessageW(editor, WM_GETTEXTLENGTH, None, None).0 as usize;
        let mut actual = vec![0u16; length + 1];
        SendMessageW(
            editor,
            WM_GETTEXT,
            Some(WPARAM(actual.len())),
            Some(LPARAM(actual.as_mut_ptr() as isize)),
        );
        Ok(String::from_utf16(&actual[..length])?)
    }
}
#[cfg(feature = "acceptance")]
#[allow(clippy::too_many_arguments)]
fn exchange(
    pipe: &Pipe,
    capability: &str,
    task: &str,
    turn: u64,
    operation: u64,
    revision: u64,
    action: Action,
    text: &str,
    expired: bool,
) -> Result<Reply, Box<dyn std::error::Error>> {
    let now = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
    let r = Request {
        revoke: None,
        block: None,
        engine: fixture_binding(),
        protocol: sayweave_text_service::wire::PROTOCOL,
        capability: capability.into(),
        task: task.into(),
        turn,
        operation,
        revision,
        deadline_tick: if expired { now - 1 } else { now + 2000 },
        action,
        text: text.into(),
        prefix: None,
    };
    pipe.send(&serde_json::to_vec(&r)?)?;
    let start = Instant::now();
    loop {
        pump(Duration::from_millis(10));
        if let Some(data) = pipe.receive()? {
            let reply: Reply = serde_json::from_slice(&data)?;
            if reply.operation != operation {
                return Err("acknowledgement operation mismatch".into());
            }
            return Ok(reply);
        }
        if start.elapsed() > Duration::from_secs(2) {
            return Err("owned acknowledgement timeout".into());
        }
    }
}
#[cfg(feature = "acceptance")]
fn generated_text_matches(actual: &str, expected: &str) -> bool {
    actual
        .replace("\r\n", "\n")
        .replace(['\r', '\u{2028}', '\u{b}'], "\n")
        == expected.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(feature = "acceptance")]
fn prefix_fixture(
    editor: windows::Win32::Foundation::HWND,
    manager: &ITfThreadMgr,
    pipe: &Pipe,
    capability: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows::{Win32::Foundation::*, core::w};
    for (committed, tail_text, move_caret) in [
        ("整理前段🙂", "尾🙂", false),
        ("整理前段🙂", "\n尾🙂", false),
        ("整理前段🙂", "\n", false),
        ("整理\n前段🙂", "尾🙂", false),
        ("整理前段🙂", "", false),
        ("整理前段🙂", "", true),
    ] {
        unsafe {
            SendMessageW(
                editor,
                WM_SETTEXT,
                None,
                Some(LPARAM(w!("左原选区右").as_ptr() as isize)),
            );
            SendMessageW(editor, 0x00B1, Some(WPARAM(1)), Some(LPARAM(4)));
        }
        // Drain the fixture's own external reset before claiming a new range.
        pump(Duration::from_millis(50));
        let task = uuid::Uuid::new_v4().to_string();
        let begin = exchange(pipe, capability, &task, 1, 1, 0, Action::Begin, "", false)?;
        if !begin.applied {
            return Err(format!("prefix begin rejected: {}", begin.code).into());
        }
        let context = unsafe { manager.GetFocus()?.GetTop()? };
        println!(
            "prefix fixture begin epoch={} context_selected=true",
            begin.range_epoch
        );
        let initial = exchange(
            pipe,
            capability,
            &task,
            1,
            2,
            1,
            Action::Update,
            "原文尾🙂",
            false,
        )?;
        if !initial.applied {
            return Err("prefix initial update failed".into());
        }
        println!(
            "prefix initial epoch={} applied={}",
            initial.range_epoch, initial.applied
        );
        if composition_text(&context).is_err() {
            return Err(
                "prefix fixture initial composition already missing before settlement".into(),
            );
        }
        let settle = Request {
            revoke: None,
            block: None,
            engine: fixture_binding(),
            protocol: PROTOCOL,
            capability: capability.into(),
            task: task.clone(),
            turn: 1,
            operation: 3,
            revision: 2,
            deadline_tick: unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
                + 2000,
            action: Action::Settle,
            text: tail_text.into(),
            prefix: Some(Prefix {
                text: committed.into(),
                expected_revision: 1,
                range_epoch: begin.range_epoch,
            }),
        };
        // The old trailing-LF request must be rejected without touching the
        // live composition, target text, caret or operation version.
        let rejected_request = Request {
            revoke: None,
            block: None,
            prefix: Some(Prefix {
                text: format!("{committed}\n"),
                expected_revision: 1,
                range_epoch: begin.range_epoch,
            }),
            ..settle.clone()
        };
        let before_rejection = editor_text(editor)?;
        let caret_before_rejection = editor_selection(editor);
        pipe.send(&serde_json::to_vec(&rejected_request)?)?;
        let rejection_started = Instant::now();
        let rejected = loop {
            pump(Duration::from_millis(10));
            if let Some(data) = pipe.receive()? {
                break serde_json::from_slice::<Reply>(&data)?;
            }
            if rejection_started.elapsed() > Duration::from_secs(2) {
                return Err("boundary rejection timeout".into());
            }
        };
        if rejected.applied
            || rejected.code != "prefixBoundaryUnsupported"
            || rejected.mutation != Mutation::NotAttempted
            || editor_text(editor)? != before_rejection
            || editor_selection(editor) != caret_before_rejection
            || composition_text(&context).is_err()
        {
            return Err("invalid boundary modified or retired the live draft".into());
        }
        println!("owned trailing-LF boundary rejected with actual text/caret unchanged=true");
        pipe.send(&serde_json::to_vec(&settle)?)?;
        let started = Instant::now();
        let reply = loop {
            pump(Duration::from_millis(10));
            if let Some(data) = pipe.receive()? {
                break serde_json::from_slice::<Reply>(&data)?;
            }
            if started.elapsed() > Duration::from_secs(2) {
                return Err("prefix response timeout".into());
            }
        };
        if reply.operation != 3
            || !reply.applied
            || !reply.settlement.is_some_and(|s| {
                s.prefix_operation == 3 && s.prefix == Mutation::Changed && s.tail_owned
            })
        {
            return Err(format!(
                "prefix settle rejected: {} mutation={:?} settlement={:?} owned_fixture_text={:?} owned_selection={:?}",
                reply.code, reply.mutation, reply.settlement, editor_text(editor)?, editor_selection(editor)
            )
            .into());
        }
        println!(
            "prefix reply epoch={} same_context={} settlement={:?}",
            reply.range_epoch,
            unsafe { manager.GetFocus()?.GetTop()? } == context,
            reply.settlement
        );
        if !tail_text.is_empty() && !generated_text_matches(&composition_text(&context)?, tail_text)
        {
            return Err(format!(
                "owned prefix range expected={tail_text:?} actual={:?}",
                composition_text(&context)?
            )
            .into());
        }
        if tail_text.is_empty() && composition_text(&context).is_ok() {
            return Err("empty tail retained a dummy composition".into());
        }
        let prefix = format!("左{committed}");
        if !editor_text(editor)?
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .starts_with(&prefix)
        {
            return Err("settled prefix is not actual committed control text".into());
        }
        if move_caret {
            let before = editor_text(editor)?;
            let before_selection = editor_selection(editor);
            unsafe {
                SendMessageW(editor, 0x00B1, Some(WPARAM(0)), Some(LPARAM(0)));
            }
            pump(Duration::from_millis(50));
            if before_selection == (0, 0) || editor_selection(editor) != (0, 0) {
                return Err("owned fixture did not move the actual caret to0".into());
            }
            let rejected = exchange(
                pipe,
                capability,
                &task,
                1,
                4,
                3,
                Action::Update,
                "不得写入",
                false,
            )?;
            if rejected.applied
                || rejected.mutation != Mutation::NotAttempted
                || editor_text(editor)? != before
            {
                return Err("empty insertion lease survived a real caret move".into());
            }
            println!(
                "owned empty-tail caret change revokes continuation with unchanged actual text=true"
            );
            continue;
        }
        if tail_text == "\n" {
            let finished = exchange(pipe, capability, &task, 1, 4, 2, Action::Commit, "", false)?;
            if !finished.applied
                || editor_text(editor)?
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    != format!("{prefix}\n右")
            {
                return Err("separator-only final draft lost its real paragraph".into());
            }
            println!("owned separator-only final draft commits exact paragraph=true");
            continue;
        }
        let carried_separator = if tail_text.starts_with('\n') {
            "\n"
        } else {
            ""
        };
        let continued = format!("{carried_separator}尾部继续🙂");
        let next_owned_tail = if tail_text.starts_with('\n') {
            "\n待撤销尾部"
        } else {
            "待撤销尾部"
        };
        let updated = exchange(
            pipe,
            capability,
            &task,
            1,
            4,
            3,
            Action::Update,
            &continued,
            false,
        )?;
        if !updated.applied || !generated_text_matches(&composition_text(&context)?, &continued) {
            return Err(format!(
                "prefix tail update failed: code={} applied={} mutation={:?}",
                updated.code, updated.applied, updated.mutation
            )
            .into());
        }
        let next = Request {
            revoke: None,
            block: None,
            operation: 5,
            revision: 4,
            deadline_tick: unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
                + 2000,
            text: next_owned_tail.into(),
            prefix: Some(Prefix {
                text: continued.clone(),
                expected_revision: 3,
                range_epoch: updated.range_epoch,
            }),
            ..settle
        };
        pipe.send(&serde_json::to_vec(&next)?)?;
        let started = Instant::now();
        let second = loop {
            pump(Duration::from_millis(10));
            if let Some(data) = pipe.receive()? {
                break serde_json::from_slice::<Reply>(&data)?;
            }
            if started.elapsed() > Duration::from_secs(2) {
                return Err("second prefix response timeout".into());
            }
        };
        if second.operation != 5
            || !second.applied
            || !second.settlement.is_some_and(|s| {
                s.prefix_operation == 5 && s.prefix == Mutation::Changed && s.tail_owned
            })
        {
            return Err(format!("second prefix rejected: {}", second.code).into());
        }
        if !generated_text_matches(&composition_text(&context)?, next_owned_tail) {
            return Err("second prefix did not retain latest tail".into());
        }
        let aborted = exchange(pipe, capability, &task, 1, 6, 4, Action::Abort, "", false)?;
        if !aborted.applied {
            return Err(format!("prefix tail abort rejected: {}", aborted.code).into());
        }
        if editor_text(editor)?
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            != format!("{prefix}{continued}右")
        {
            return Err(
                "prefix cancellation erased committed text or restored replaced selection".into(),
            );
        }
    }
    println!("owned prefix settle, live tail, Unicode/newline and cancel actual text=true");
    Ok(())
}
#[cfg(feature = "acceptance")]
fn trace_native_initial_settlement(
    editor: windows::Win32::Foundation::HWND,
    context: &ITfContext,
    pipe: &Pipe,
    capability: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows::{Win32::Foundation::*, core::w};
    unsafe {
        for moved in [false, true] {
            SendMessageW(
                editor,
                WM_SETTEXT,
                None,
                Some(LPARAM(w!("左原选区右").as_ptr() as isize)),
            );
            SendMessageW(editor, 0xB1, Some(WPARAM(1)), Some(LPARAM(4)));
            pump(Duration::from_millis(100));
            let task = uuid::Uuid::new_v4().to_string();
            let snapshot = |stage: &str, reply: Option<&Reply>| {
                println!(
                    "{}",
                    serde_json::json!({
                        "owned_edit_trace":true,"moved":moved,"stage":stage,
                        "reply":reply.map(|r|serde_json::json!({"applied":r.applied,"mutation":format!("{:?}",r.mutation),"code":r.code})),
                        "control":editor_text(editor).ok(),"selection":editor_selection(editor),
                        "composition":composition_text(context).ok()
                    })
                );
            };
            snapshot("beforeBegin", None);
            let begin = exchange(pipe, capability, &task, 1, 1, 0, Action::Begin, "", false)?;
            snapshot("afterBegin", Some(&begin));
            if moved {
                SendMessageW(editor, 0xB1, Some(WPARAM(0)), Some(LPARAM(0)));
            }
            snapshot("beforeUpdate", None);
            let update = exchange(
                pipe,
                capability,
                &task,
                1,
                2,
                1,
                Action::Update,
                "旧结果",
                false,
            )?;
            snapshot("afterUpdate", Some(&update));
            pump(Duration::from_millis(500));
            snapshot("afterWait", None);
            let commit = exchange(pipe, capability, &task, 1, 3, 1, Action::Commit, "", false)?;
            snapshot("afterCommit", Some(&commit));
            pump(Duration::from_millis(500));
            snapshot("afterCommitWait", None);
            if !moved
                && (!update.applied || !commit.applied || editor_text(editor)? != "左旧结果右")
            {
                return Err("owned EDIT unchanged-selection control failed".into());
            }
            if moved
                && std::env::var("SAYWEAVE_EDIT_SETTLEMENT_TRACE").as_deref() == Ok("assert")
                && (update.applied
                    || update.mutation != Mutation::NotAttempted
                    || editor_text(editor)? != "左原选区右"
                    || editor_selection(editor) != (0, 0))
            {
                return Err(
                    "owned EDIT moved-selection changed immediately or after finalization".into(),
                );
            }
        }
    }
    Ok(())
}

#[cfg(feature = "acceptance")]
fn prepared_native_selection_fixture(
    editor: windows::Win32::Foundation::HWND,
    pipe: &Pipe,
    capability: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows::{Win32::Foundation::*, core::w};
    unsafe {
        SendMessageW(
            editor,
            WM_SETTEXT,
            None,
            Some(LPARAM(w!("左原选区右").as_ptr() as isize)),
        );
        SendMessageW(editor, 0xB1, Some(WPARAM(1)), Some(LPARAM(4)));
        pump(Duration::from_millis(50));
        let task = uuid::Uuid::new_v4().to_string();
        let begin = exchange(pipe, capability, &task, 1, 1, 0, Action::Begin, "", false)?;
        if !begin.applied || editor_text(editor)? != "左原选区右" {
            return Err("native selected Begin failed".into());
        }
        // A programmatic native selection change emits no physical key event.
        SendMessageW(editor, 0xB1, Some(WPARAM(0)), Some(LPARAM(0)));
        let update = exchange(
            pipe,
            capability,
            &task,
            1,
            2,
            1,
            Action::Update,
            "旧结果",
            false,
        )?;
        if update.applied
            || update.mutation != Mutation::NotAttempted
            || editor_text(editor)? != "左原选区右"
            || editor_selection(editor) != (0, 0)
        {
            return Err(format!("native prepared selection protection failed applied={} mutation={:?} code={} text_unchanged={} selection={:?}", update.applied, update.mutation, update.code, editor_text(editor)? == "左原选区右", editor_selection(editor)).into());
        }
        pump(Duration::from_millis(250));
        let commit = exchange(pipe, capability, &task, 1, 3, 1, Action::Commit, "", false)?;
        pump(Duration::from_millis(250));
        if commit.applied
            || editor_text(editor)? != "左原选区右"
            || editor_selection(editor) != (0, 0)
        {
            return Err("native rejected first write landed during finalization".into());
        }
        println!(
            "owned native prepared selection change rejects old write with actual text/caret unchanged=true"
        );
    }
    Ok(())
}

#[cfg(feature = "acceptance")]
fn initial_selection_fixture(
    component: windows::Win32::Foundation::HMODULE,
    editor: windows::Win32::Foundation::HWND,
    pipe: &Pipe,
    capability: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows::{
        Win32::{Foundation::*, System::LibraryLoader::*},
        core::{s, w},
    };
    unsafe {
        let reset = || {
            SendMessageW(
                editor,
                WM_SETTEXT,
                None,
                Some(LPARAM(w!("左原选区右").as_ptr() as isize)),
            );
            SendMessageW(editor, 0xB1, Some(WPARAM(1)), Some(LPARAM(4)));
            pump(Duration::from_millis(50));
        };
        let receive = || -> Result<Reply, Box<dyn std::error::Error>> {
            let start = Instant::now();
            loop {
                pump(Duration::from_millis(10));
                if let Some(data) = pipe.receive()? {
                    return Ok(serde_json::from_slice(&data)?);
                }
                if start.elapsed() > Duration::from_secs(2) {
                    return Err("initial fixture timeout".into());
                }
            }
        };
        for action in [Action::Abort, Action::Commit, Action::Handoff] {
            reset();
            let task = uuid::Uuid::new_v4().to_string();
            let begin = exchange(pipe, capability, &task, 1, 1, 0, Action::Begin, "", false)?;
            if !begin.applied || begin.range_epoch == 0 || editor_text(editor)? != "左原选区右"
            {
                return Err("prepared Begin changed text or lost epoch".into());
            }
            let empty = exchange(pipe, capability, &task, 1, 2, 1, Action::Update, "", false)?;
            let end = exchange(pipe, capability, &task, 1, 3, 1, action, "", false)?;
            if !empty.applied
                || empty.mutation != Mutation::NotAttempted
                || !end.applied
                || end.mutation != Mutation::NotAttempted
                || editor_text(editor)? != "左原选区右"
            {
                return Err("prepared terminal changed original text".into());
            }
        }
        let entry = GetProcAddress(component, s!("SayweaveInitialFixtureFault"))
            .ok_or("missing initial fault fixture")?;
        let inject: unsafe extern "system" fn(usize) = std::mem::transmute(entry);
        for mode in 0..=3 {
            reset();
            let task = uuid::Uuid::new_v4().to_string();
            let begin = exchange(pipe, capability, &task, 1, 1, 0, Action::Begin, "", false)?;
            if !begin.applied || begin.range_epoch == 0 {
                return Err("initial fault Begin failed".into());
            }
            inject(mode);
            let request = Request {
                revoke: None,
                block: None,
                engine: fixture_binding(),
                protocol: sayweave_text_service::wire::PROTOCOL,
                capability: capability.into(),
                task: task.clone(),
                turn: 1,
                operation: 2,
                revision: 1,
                deadline_tick: windows::Win32::System::SystemInformation::GetTickCount64() + 2000,
                action: if mode <= 1 {
                    Action::Settle
                } else {
                    Action::Update
                },
                text: "尾".into(),
                prefix: (mode <= 1).then(|| Prefix {
                    text: "整理".into(),
                    expected_revision: 0,
                    range_epoch: begin.range_epoch,
                }),
            };
            pipe.send(&serde_json::to_vec(&request)?)?;
            let first = receive()?;
            match mode {
                0 => {
                    if !first.applied
                        || !first.settlement.is_some_and(|p| {
                            p.prefix_operation == 2 && p.prefix == Mutation::Changed
                        })
                        || editor_text(editor)? != "左整理尾右"
                    {
                        return Err("direct initial Settle failed".into());
                    }
                    let abort =
                        exchange(pipe, capability, &task, 1, 4, 1, Action::Abort, "", false)?;
                    if !abort.applied || editor_text(editor)? != "左整理右" {
                        return Err("initial Settle cancellation erased prefix".into());
                    }
                }
                1 => {
                    if first.applied
                        || first.mutation == Mutation::NotAttempted
                        || !first.settlement.is_some_and(|p| {
                            p.prefix_operation == 2 && p.prefix == Mutation::Uncertain
                        })
                        || editor_text(editor)? != "左原选区右"
                    {
                        return Err(
                            "initial partial failure was misclassified or not restored".into()
                        );
                    }
                }
                2 => {
                    let original = receive()?;
                    if first.operation != 3
                        || first.applied
                        || first.mutation != Mutation::Uncertain
                        || original.operation != 2
                        || !original.applied
                        || editor_text(editor)? != "左尾右"
                    {
                        return Err("in-flight initial cancellation claimed no mutation".into());
                    }
                    let abort =
                        exchange(pipe, capability, &task, 1, 4, 1, Action::Abort, "", false)?;
                    if !abort.applied || editor_text(editor)? != "左原选区右" {
                        return Err("in-flight refusal lost original backup".into());
                    }
                }
                3 => {
                    if first.operation != 3
                        || !first.applied
                        || first.mutation != Mutation::NotAttempted
                        || editor_text(editor)? != "左原选区右"
                    {
                        return Err("pre-write reentrant cancellation changed original".into());
                    }
                }
                _ => unreachable!(),
            }
            println!(
                "owned initial-selection checkpoint={mode} actual text and mutation classification=true"
            );
        }
    }
    Ok(())
}

#[cfg(feature = "acceptance")]
fn editor_selection(window: windows::Win32::Foundation::HWND) -> (u32, u32) {
    use windows::Win32::Foundation::*;
    let (mut start, mut end) = (0u32, 0u32);
    unsafe {
        SendMessageW(
            window,
            0x00B0,
            Some(WPARAM((&mut start as *mut u32) as usize)),
            Some(LPARAM((&mut end as *mut u32) as isize)),
        );
    }
    (start, end)
}
#[cfg(feature = "acceptance")]
fn prefix_fault_fixture(
    component: windows::Win32::Foundation::HMODULE,
    editor: windows::Win32::Foundation::HWND,
    pipe: &Pipe,
    capability: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows::{
        Win32::{Foundation::*, System::LibraryLoader::*},
        core::{s, w},
    };
    unsafe {
        let entry = GetProcAddress(component, s!("SayweaveSettleFixtureFault"))
            .ok_or("missing owned acceptance-only checkpoint")?;
        let inject: unsafe extern "system" fn(usize) = std::mem::transmute(entry);
        for mode in [1, 2, 3, 4, 5] {
            SendMessageW(
                editor,
                WM_SETTEXT,
                None,
                Some(LPARAM(w!("左原选区右").as_ptr() as isize)),
            );
            SendMessageW(editor, 0x00B1, Some(WPARAM(1)), Some(LPARAM(4)));
            // The fixture reset is an external edit. Deliver its notifications
            // before claiming a new range; this is not a product start delay.
            pump(Duration::from_millis(50));
            let task = uuid::Uuid::new_v4().to_string();
            let begin = exchange(pipe, capability, &task, 1, 1, 0, Action::Begin, "", false)?;
            if !begin.applied
                || !exchange(
                    pipe,
                    capability,
                    &task,
                    1,
                    2,
                    1,
                    Action::Update,
                    "原文尾🙂",
                    false,
                )?
                .applied
            {
                return Err("prefix fault fixture setup failed".into());
            }
            inject(mode);
            let raw_tail = if matches!(mode, 4 | 5) {
                "原文\n尾🙂"
            } else {
                "原文尾🙂"
            };
            let request = Request {
                revoke: None,
                block: None,
                engine: fixture_binding(),
                protocol: PROTOCOL,
                capability: capability.into(),
                task: task.clone(),
                turn: 1,
                operation: 3,
                revision: 2,
                deadline_tick: windows::Win32::System::SystemInformation::GetTickCount64() + 2000,
                action: Action::Settle,
                text: raw_tail.into(),
                prefix: Some(Prefix {
                    text: "提交前段🙂".into(),
                    expected_revision: 1,
                    range_epoch: begin.range_epoch,
                }),
            };
            pipe.send(&serde_json::to_vec(&request)?)?;
            let started = Instant::now();
            let reply = loop {
                pump(Duration::from_millis(10));
                if let Some(frame) = pipe.receive()? {
                    break serde_json::from_slice::<Reply>(&frame)?;
                }
                if started.elapsed() > Duration::from_secs(2) {
                    return Err("prefix fault reply missing".into());
                }
            };
            let cancelled = matches!(mode, 1 | 5);
            if (cancelled && (!reply.applied || reply.code != "applied"))
                || (!cancelled && reply.applied)
                || reply.operation != if cancelled { 4 } else { 3 }
            {
                return Err(format!("prefix fault mode={mode} reply applied={} operation={} code={} settlement={:?}",reply.applied,reply.operation,reply.code,reply.settlement).into());
            }
            if !reply.settlement.is_some_and(|s| {
                s.prefix_operation == 3 && s.prefix == Mutation::Changed && !s.tail_owned
            }) {
                return Err("partial prefix outcome lost its irreversible effect".into());
            }
            pump(Duration::from_millis(30));
            let actual = editor_text(editor)?
                .replace("\r\n", "\n")
                .replace('\r', "\n");
            let expected_actual = if cancelled {
                "左提交前段🙂右".to_string()
            } else {
                format!("左提交前段🙂{raw_tail}右")
            };
            if actual != expected_actual {
                return Err(
                    "post-prefix cancellation/failure erased prefix or restored original selection"
                        .into(),
                );
            }
            let stale = exchange(
                pipe,
                capability,
                &task,
                1,
                5,
                3,
                Action::Update,
                "不得重复",
                false,
            )?;
            if stale.applied
                || stale.mutation != Mutation::NotAttempted
                || editor_text(editor)?
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    != actual
            {
                return Err("partial prefix failure allowed replay into target".into());
            }
            println!(
                "owned prefix checkpoint={mode} preserves actual prefix and denies replay=true"
            );
            if mode == 3 {
                if !exchange(pipe, capability, &task, 2, 1, 0, Action::Begin, "", false)?.applied {
                    return Err(
                        "interrupted prefix left native transaction busy for the next turn".into(),
                    );
                }
                if !exchange(
                    pipe,
                    capability,
                    &task,
                    2,
                    2,
                    1,
                    Action::Update,
                    "新轮次",
                    false,
                )?
                .applied
                    || !exchange(pipe, capability, &task, 2, 3, 1, Action::Commit, "", false)?
                        .applied
                {
                    return Err("interrupted prefix next turn could not write".into());
                }
                if editor_text(editor)?
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    != "左提交前段🙂原文尾🙂新轮次右"
                {
                    return Err("interrupted prefix old tail wrote over the new turn".into());
                }
            }
        }
    }
    Ok(())
}
fn pump(duration: Duration) {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
