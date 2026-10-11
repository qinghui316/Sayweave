//! Ordinary installer controller. Elevated code never opens these app/user backups.
use super::{
    ipc, machine, metadata,
    record::{
        FileBackup, MAX_FILE, MAX_RECORD, Phase, Process, Result, Target, UserRecord, hash, read,
        recovery_owner, reject_links, write,
    },
};
use sayweave_text_service::{registration, transport::Pipe};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        System::Threading::*,
        UI::Shell::*,
    },
    core::PCWSTR,
};

pub fn target(main_source: &Path) -> Result<Target> {
    let build = crate::build_info::current();
    let exe = std::env::current_exe().map_err(|_| "installerExecutable")?;
    let value = Target {
        version: build.version.into(),
        commit: build.commit.into(),
        product: env!("SAYWEAVE_BUILD_PRODUCT").into(),
        payload_hash: hash(crate::text_service::payload()),
        main_hash: hash(&read(main_source, MAX_FILE)?),
        helper_hash: hash(&read(&exe, MAX_FILE)?),
        uninstall_hash: hash(&read(
            &exe.parent()
                .ok_or("installerExecutable")?
                .join("sayweave-uninstall.exe"),
            MAX_FILE,
        )?),
    };
    if crate::text_service::payload().is_empty() {
        return Err("componentPayloadMissing".into());
    }
    value.validate()?;
    Ok(value)
}
fn package_main(source: &str) -> Result<PathBuf> {
    let path = PathBuf::from(source);
    reject_links(&path)?;
    let helper = std::env::current_exe().map_err(|_| "installerExecutable")?;
    if path.file_name() != Some(std::ffi::OsStr::new("sayweave-main.exe"))
        || !metadata::same_path(
            &path
                .parent()
                .ok_or("installerExecutable")?
                .to_string_lossy(),
            &helper
                .parent()
                .ok_or("installerExecutable")?
                .to_string_lossy(),
        )
    {
        return Err("installerPackageSource".into());
    }
    let bytes = read(&path, MAX_FILE)?;
    if !bytes.starts_with(b"MZ")
        || !bytes
            .windows(crate::build_info::current().commit.len())
            .any(|v| v == crate::build_info::current().commit.as_bytes())
    {
        return Err("installerPackageIdentity".into());
    }
    Ok(path)
}
fn running_target() -> Result<Target> {
    let exe = std::env::current_exe().map_err(|_| "installerExecutable")?;
    let helper = exe
        .parent()
        .ok_or("installerDirectory")?
        .join("sayweave-installer-helper.exe");
    let build = crate::build_info::current();
    let value = Target {
        version: build.version.into(),
        commit: build.commit.into(),
        product: env!("SAYWEAVE_BUILD_PRODUCT").into(),
        payload_hash: hash(crate::text_service::payload()),
        main_hash: hash(&read(&exe, MAX_FILE)?),
        helper_hash: hash(&read(&helper, MAX_FILE)?),
        uninstall_hash: hash(&read(
            &exe.parent()
                .ok_or("installerDirectory")?
                .join("uninstall.exe"),
            MAX_FILE,
        )?),
    };
    value.validate()?;
    Ok(value)
}
pub struct MachineLock(Pipe);
impl MachineLock {
    pub fn acquire() -> Result<Self> {
        unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(metadata::wide("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x100001;;;BU)").as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
            .map_err(|_| "installerLockSecurity")?;
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            };
            let handle = CreateMutexExW(
                Some(&attributes),
                PCWSTR(metadata::wide("Global\\Sayweave.TSF.Installer.v1").as_ptr()),
                0,
                0x100001,
            )
            .map_err(|_| "installerLock")?;
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
            let handle = Pipe(handle);
            if !matches!(
                WaitForSingleObject(handle.0, 0),
                WAIT_OBJECT_0 | WAIT_ABANDONED
            ) {
                return Err("installerBusy".into());
            }
            Ok(Self(handle))
        }
    }
}
impl Drop for MachineLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0.0);
        }
    }
}
fn arguments(args: &[String]) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for arg in args.iter().skip(1) {
        let (name, value) = arg
            .strip_prefix("--")
            .and_then(|v| v.split_once('='))
            .ok_or("installerArguments")?;
        if value.is_empty()
            || value.len() > 4096
            || values.insert(name.into(), value.into()).is_some()
        {
            return Err("installerArguments".into());
        }
    }
    Ok(values)
}
fn required<'a>(values: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str> {
    values
        .get(name)
        .map(String::as_str)
        .ok_or("installerArguments".into())
}
pub fn dispatch(args: &[String]) -> Result<()> {
    metadata::initialize_apartment()?;
    let action = args[0]
        .strip_prefix("--sayweave-installer=")
        .ok_or("installerArguments")?;
    let values = arguments(args)?;
    if action == "server" {
        if values.len() != 8 {
            return Err("installerArguments".into());
        }
        let pid = required(&values, "installer-pid")?
            .parse::<u32>()
            .map_err(|_| "installerArguments")?;
        let installer = ipc::process(pid)?;
        if installer.created.to_string() != required(&values, "installer-created")?
            || installer.sid != required(&values, "initiator")?
            || installer.image_hash != required(&values, "installer-hash")?
        {
            return Err("installerProcessChanged".into());
        }
        return machine::serve(
            required(&values, "transaction")?,
            installer,
            required(&values, "language")?
                .parse()
                .map_err(|_| "installerArguments")?,
            required(&values, "recover")? == "1",
            &package_main(required(&values, "main-source")?)?,
        );
    }
    if !matches!(
        action,
        "begin" | "check" | "commit" | "rollback" | "uninstall"
    ) || values.len() != if action == "uninstall" { 3 } else { 4 }
    {
        return Err("installerArguments".into());
    }
    let directory = PathBuf::from(required(&values, "install-dir")?);
    reject_links(&directory)?;
    if directory.parent().is_none() || directory.file_name().is_none() {
        return Err("installerDirectory".into());
    }
    // Serialize receipt publication too, before an elevated machine lock exists.
    let _receipt_lock = receipt_lock(&directory)?;
    let product = env!("SAYWEAVE_BUILD_PRODUCT");
    let keys = metadata::keys(product, required(&values, "product-key")?)?;
    let pid = required(&values, "installer-pid")?
        .parse()
        .map_err(|_| "installerArguments")?;
    let process = ipc::process(pid)?;
    if process.sid != metadata::user_sid()? {
        return Err("installerInitiator".into());
    }
    if action == "uninstall" {
        return uninstall(&directory, keys, &process);
    }
    if process.image_hash == target(&package_main(required(&values, "main-source")?)?)?.helper_hash
    {
        return Err("installerInitiator".into());
    }
    let target = target(&package_main(required(&values, "main-source")?)?)?;
    match action {
        "begin" => begin(&directory, keys, process, target),
        "check" => {
            let (_, record) = current_record(&directory, &process, &target)?;
            let machine = machine_for(&record)?;
            if machine.phase != Phase::Prepared || !ipc::live(&machine.server) {
                return Err("installerSessionEnded".into());
            }
            if ipc::running_at(&directory.join("sayweave.exe"))? {
                Err("installerAppRunning".into())
            } else {
                Ok(())
            }
        }
        "commit" => commit(&directory, &process, &target),
        "rollback" => rollback(&directory, &process),
        _ => unreachable!(),
    }
}
fn record_dir(directory: &Path) -> Result<PathBuf> {
    reject_links(directory)?;
    let name = directory
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .trim_end_matches('\\')
        .to_lowercase();
    let root = metadata::folder(&FOLDERID_LocalAppData)?
        .join("SayweaveInstallTransactions")
        .join(hash(name.as_bytes()));
    reject_links(&root)?;
    Ok(root)
}
fn receipt_lock(directory: &Path) -> Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    let root = record_dir(directory)?;
    fs::create_dir_all(&root).map_err(|_| "installerWriteFailed")?;
    let path = root.join("lock");
    reject_links(&path)?;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
        .map_err(|_| "installerBusy".into())
}
fn save(root: &Path, record: &UserRecord) -> Result<()> {
    record.validate()?;
    validate_paths(record)?;
    let bytes = serde_json::to_vec(record).map_err(|_| "installerRecordInvalid")?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err("installerRecordLimit".into());
    }
    write(&root.join("receipt.json"), &bytes)
}
fn load(root: &Path) -> Result<Option<UserRecord>> {
    let path = root.join("receipt.json");
    if !path.exists() {
        return Ok(None);
    }
    let record: UserRecord =
        serde_json::from_slice(&read(&path, MAX_RECORD)?).map_err(|_| "installerRecordInvalid")?;
    record.validate()?;
    validate_paths(&record)?;
    Ok(Some(record))
}
fn file_paths(directory: &Path, product: &str) -> Result<[(PathBuf, &'static str); 5]> {
    Ok([
        (directory.join("sayweave.exe"), "main"),
        (directory.join("uninstall.exe"), "uninstall"),
        (directory.join("sayweave-installer-helper.exe"), "helper"),
        (
            metadata::folder(&FOLDERID_Desktop)?.join(format!("{product}.lnk")),
            "desktop",
        ),
        (
            metadata::folder(&FOLDERID_Programs)?.join(format!("{product}.lnk")),
            "startMenu",
        ),
    ])
}
fn validate_paths(record: &UserRecord) -> Result<()> {
    let directory = PathBuf::from(&record.install_dir);
    reject_links(&directory)?;
    let paths = file_paths(&directory, &record.target.product)?;
    if record.files.len() != paths.len() {
        return Err("installerBackupInvalid".into());
    }
    for ((path, name), file) in paths.iter().zip(&record.files) {
        if *name != file.backup || !metadata::same_path(&path.to_string_lossy(), &file.path) {
            return Err("installerBackupInvalid".into());
        }
    }
    let expected = metadata::keys(&record.target.product, &record.keys[0].path)?;
    if record.keys[1].path != expected[1] {
        return Err("installerBackupInvalid".into());
    }
    Ok(())
}
fn backup(root: &Path, path: &Path, name: &str, transaction: &str) -> Result<FileBackup> {
    reject_links(path)?;
    let before_hash = if path.exists() {
        let data = read(path, MAX_FILE)?;
        let identity = hash(&data);
        write(&root.join(transaction).join(name), &data)?;
        Some(identity)
    } else {
        None
    };
    Ok(FileBackup {
        path: path.to_string_lossy().into_owned(),
        backup: name.into(),
        before_hash,
        installed_hash: None,
    })
}
fn stage_server(record: &UserRecord, recover: bool) -> Result<machine::MachineRecord> {
    let args = format!(
        "--sayweave-installer=server --transaction={} --installer-pid={} --installer-created={} --initiator={} --installer-hash={} --language={} --recover={} \"--main-source={}\"",
        record.transaction,
        record.installer.pid,
        record.installer.created,
        record.installer.sid,
        record.installer.image_hash,
        record.language,
        u8::from(recover),
        std::env::current_exe()
            .map_err(|_| "installerExecutable")?
            .parent()
            .ok_or("installerExecutable")?
            .join("sayweave-main.exe")
            .to_string_lossy()
    );
    let child = ipc::elevate(&args)?;
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(value) = machine::load()?
            && value.transaction == record.transaction
            && value.installer == record.installer
            && ipc::live(&value.server)
        {
            ipc::request(&value, "prepared")?;
            return Ok(value);
        }
        if unsafe { WaitForSingleObject(child.0, 0) } != WAIT_TIMEOUT || Instant::now() > deadline {
            return Err("installerMaintenanceFailed".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn begin(
    directory: &Path,
    key_paths: [String; 2],
    installer: Process,
    target: Target,
) -> Result<()> {
    let root = record_dir(directory)?;
    if let Some(mut prior) = load(&root)?
        && !prior.phase.terminal()
    {
        recover(&root, &mut prior, &installer)?;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while ipc::running_at(&directory.join("sayweave.exe"))? {
        if Instant::now() > deadline {
            return Err("installerAppRunning".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let key_snapshots = key_paths
        .iter()
        .map(|key| metadata::snapshot(key))
        .collect::<Result<Vec<_>>>()?;
    if !key_snapshots[1].existed
        && (directory.join("sayweave.exe").exists()
            || directory.join("uninstall.exe").exists()
            || directory.join("sayweave-installer-helper.exe").exists())
    {
        return Err("installerUnknownProgram".into());
    }
    if let Some(location) = metadata::install_location(&key_snapshots[1])?
        && !metadata::same_path(&location, &directory.to_string_lossy())
    {
        return Err("installerOriginalPathRequired".into());
    }
    let transaction = uuid::Uuid::new_v4().to_string();
    if let Some(old) = machine::load()?
        && old.phase == Phase::Committed
        && old.target.payload_hash != target.payload_hash
        && version(&old.target.version) >= version(&target.version)
    {
        return Err("componentDowngradeRejected".into());
    }
    let mut files = Vec::new();
    for (path, name) in file_paths(directory, &target.product)? {
        if path.exists()
            && matches!(name, "desktop" | "startMenu")
            && !metadata::shortcut_owned(&path, &directory.to_string_lossy())?
        {
            return Err("installerShortcutChanged".into());
        }
        let mut file = backup(&root, &path, name, &transaction)?;
        file.installed_hash = match name {
            "main" => Some(target.main_hash.clone()),
            "uninstall" => Some(target.uninstall_hash.clone()),
            "helper" => Some(target.helper_hash.clone()),
            _ => None,
        };
        files.push(file);
    }
    let language = metadata::current_language()?;
    let mut record = UserRecord {
        schema_version: 1,
        transaction,
        phase: Phase::BackedUp,
        target,
        installer,
        server: None,
        install_dir: directory.to_string_lossy().into_owned(),
        language,
        user_choice: registration::installer_user_choice(language)
            .map_err(|_| "componentUserChoice")?,
        user_initialization_started: false,
        user_initialized: false,
        files,
        keys: key_snapshots,
        error: None,
    };
    save(&root, &record)?;
    match stage_server(&record, false) {
        Ok(machine) => {
            record.server = Some(machine.server);
            record.phase = record.phase.advance(Phase::Prepared)?;
            save(&root, &record)
        }
        Err(error) => {
            record.phase = Phase::Recovery;
            record.error = Some(error.clone());
            save(&root, &record)?;
            Err(error)
        }
    }
}
fn current_record(
    directory: &Path,
    installer: &Process,
    target: &Target,
) -> Result<(PathBuf, UserRecord)> {
    let root = record_dir(directory)?;
    let record = load(&root)?.ok_or("installerRecordMissing")?;
    if record.installer != *installer || record.target != *target {
        return Err("installerProcessChanged".into());
    }
    Ok((root, record))
}
fn installed_files(record: &mut UserRecord) -> Result<()> {
    for file in &mut record.files {
        let path = PathBuf::from(&file.path);
        file.installed_hash = if path.exists() {
            Some(hash(&read(&path, MAX_FILE)?))
        } else {
            None
        };
    }
    if record.files[0].installed_hash.as_deref() != Some(record.target.main_hash.as_str())
        || record.files[1].installed_hash.as_deref() != Some(record.target.uninstall_hash.as_str())
        || record.files[2].installed_hash.as_deref() != Some(record.target.helper_hash.as_str())
    {
        return Err("installerProgramIdentity".into());
    }
    let location = metadata::uninstall_owned_location(&record.target.product)?
        .ok_or("installerRegistryIdentity")?;
    if !metadata::same_path(&location, &record.install_dir) {
        return Err("installerRegistryIdentity".into());
    }
    Ok(())
}
fn machine_for(record: &UserRecord) -> Result<machine::MachineRecord> {
    let value = machine::load()?.ok_or("installerMachineMissing")?;
    if value.transaction != record.transaction
        || value.target != record.target
        || value.installer != record.installer
    {
        return Err("installerMachineChanged".into());
    }
    Ok(value)
}
fn commit(directory: &Path, installer: &Process, target: &Target) -> Result<()> {
    let (root, mut record) = current_record(directory, installer, target)?;
    if record.phase == Phase::Committed {
        return verify_ready(&record);
    }
    let result = (|| {
        installed_files(&mut record)?;
        record.phase = record.phase.advance(Phase::ProgramInstalled)?;
        save(&root, &record)?;
        #[cfg(feature = "desktop-acceptance")]
        if std::env::var("SAYWEAVE_INSTALL_ACCEPTANCE_FAULT").as_deref() == Ok("after-program") {
            return Err("acceptanceInstallerProgramFailure".into());
        }
        let machine = ipc::request(&machine_for(&record)?, "commit")?;
        record.phase = record.phase.advance(Phase::MachineCommitted)?;
        // Persist intent before the first HKCU effect, including failed API verification.
        record.user_initialization_started = true;
        save(&root, &record)?;
        registration::initialize_installer_user(record.language, record.user_choice)
            .map_err(|_| "componentUserInitialize")?;
        record.user_initialized = true;
        record.phase = record.phase.advance(Phase::UserInitialized)?;
        save(&root, &record)?;
        ipc::request(&machine, "finalize")?;
        record.phase = record.phase.advance(Phase::Committed)?;
        record.error = None;
        save(&root, &record)?;
        verify_ready(&record)
    })();
    if let Err(error) = result {
        record.error = Some(error.clone());
        record.phase = Phase::Recovery;
        save(&root, &record)?;
        return Err(error);
    }
    Ok(())
}
fn restore_files(root: &Path, record: &UserRecord) -> Result<()> {
    // Validate all identities before any restoration. Unknown edits retain the recovery anchor.
    for file in &record.files {
        let path = PathBuf::from(&file.path);
        if let Some(expected) = &file.before_hash
            && hash(&read(
                &root.join(&record.transaction).join(&file.backup),
                MAX_FILE,
            )?) != *expected
        {
            return Err("installerBackupChanged".into());
        }
        if path.exists() {
            let current = hash(&read(&path, MAX_FILE)?);
            let known = file.before_hash.as_deref() == Some(current.as_str())
                || file.installed_hash.as_deref() == Some(current.as_str())
                || file.backup == "main" && current == record.target.main_hash
                || file.backup == "helper" && current == record.target.helper_hash
                || matches!(file.backup.as_str(), "desktop" | "startMenu")
                    && metadata::shortcut_owned(&path, &record.install_dir)?;
            if !known {
                return Err("installerProgramChanged".into());
            }
        }
    }
    for file in &record.files {
        let path = PathBuf::from(&file.path);
        if let Some(before) = &file.before_hash {
            if !path.exists() || hash(&read(&path, MAX_FILE)?) != *before {
                write(
                    &path,
                    &read(&root.join(&record.transaction).join(&file.backup), MAX_FILE)?,
                )?;
            }
        } else if path.exists() {
            fs::remove_file(&path).map_err(|_| "installerRestoreFailed")?;
        }
    }
    for key in &record.keys {
        metadata::restore(key, &record.install_dir)?;
    }
    for file in &record.files {
        let path = PathBuf::from(&file.path);
        let actual = if path.exists() {
            Some(hash(&read(&path, MAX_FILE)?))
        } else {
            None
        };
        if actual != file.before_hash {
            return Err("installerRestoreFailed".into());
        }
    }
    Ok(())
}
fn recover(root: &Path, record: &mut UserRecord, installer: &Process) -> Result<()> {
    recovery_owner(&record.installer, installer, ipc::live(&record.installer))?;
    let mut machine = machine::load()?;
    if let Some(value) = &machine
        && (value.transaction != record.transaction || value.target != record.target)
    {
        if record.server.is_none() && value.phase.terminal() {
            machine = None;
        } else {
            return Err("installerRecoveryConflict".into());
        }
    }
    if let Some(value) = &machine
        && value.phase == Phase::Committed
    {
        verify_ready(record)?;
        record.phase = Phase::Committed;
        record.error = None;
        return save(root, record);
    }
    if record.user_initialization_started {
        registration::restore_installer_user(record.language, record.user_choice)
            .map_err(|_| "componentUserRestore")?;
    }
    if let Some(value) = &machine
        && value.phase != Phase::RolledBack
    {
        if !ipc::live(&value.server) {
            record.installer = installer.clone();
            save(root, record)?;
            machine = Some(stage_server(record, true)?);
        }
        ipc::request(machine.as_ref().unwrap(), "rollback")?;
    }
    restore_files(root, record)?;
    record.phase = Phase::RolledBack;
    record.error = None;
    save(root, record)
}
fn rollback(directory: &Path, installer: &Process) -> Result<()> {
    let root = record_dir(directory)?;
    let Some(mut record) = load(&root)? else {
        return Ok(());
    };
    if record.phase.terminal() {
        return Ok(());
    }
    if record.installer.sid != installer.sid {
        return Err("installerInitiator".into());
    }
    recover(&root, &mut record, installer)
}
fn uninstall(directory: &Path, keys: [String; 2], process: &Process) -> Result<()> {
    let _machine_lock = MachineLock::acquire()?;
    let root = record_dir(directory)?;
    let original = load(&root)?.ok_or("installerOwnershipMissing")?;
    if original.phase != Phase::Committed || original.installer.sid != process.sid {
        return Err("installerRecoveryRequired".into());
    }
    if matches!(machine::load()?, Some(ref value) if !value.phase.terminal()) {
        return Err("installerBusy".into());
    }
    let helper = std::env::current_exe().map_err(|_| "installerExecutable")?;
    if process.image_hash != original.target.uninstall_hash
        || hash(&read(&helper, MAX_FILE)?) != original.target.helper_hash
        || original.target.product != env!("SAYWEAVE_BUILD_PRODUCT")
        || original.target.commit != crate::build_info::current().commit
    {
        return Err("installerProgramIdentity".into());
    }
    if ipc::running_at(&directory.join("sayweave.exe"))? {
        return Err("installerAppRunning".into());
    }
    for file in &original.files {
        let path = PathBuf::from(&file.path);
        if path.exists()
            && Some(hash(&read(&path, MAX_FILE)?)) != file.installed_hash
            && !(matches!(file.backup.as_str(), "desktop" | "startMenu")
                && metadata::shortcut_owned(&path, &original.install_dir)?)
        {
            return Err("installerProgramChanged".into());
        }
    }
    let transaction = uuid::Uuid::new_v4().to_string();
    let mut files = Vec::new();
    for (path, name) in file_paths(directory, &original.target.product)? {
        files.push(backup(&root, &path, name, &transaction)?);
    }
    let mut record = UserRecord {
        transaction,
        phase: Phase::Uninstalling,
        files,
        keys: keys
            .iter()
            .map(|key| metadata::snapshot(key))
            .collect::<Result<Vec<_>>>()?,
        installer: process.clone(),
        server: None,
        user_initialization_started: false,
        user_initialized: false,
        error: None,
        ..original.clone()
    };
    save(&root, &record)?;
    let result = (|| {
        for file in &record.files {
            let path = PathBuf::from(&file.path);
            if path.exists() {
                fs::remove_file(path).map_err(|_| "installerUninstallFailed")?;
            }
        }
        for key in &record.keys {
            metadata::remove_owned_key(key, &record.install_dir)?;
        }
        record.phase = Phase::Uninstalled;
        save(&root, &record)
    })();
    if let Err(error) = result {
        record.phase = Phase::Recovery;
        record.error = Some(error.clone());
        save(&root, &record)?;
        recover(&root, &mut record, process)?;
        save(&root, &original)?;
        return Err(error);
    }
    // The machine profile is shared across Windows users. A per-user uninstall
    // cannot prove it has no other consumers, so retain its registration/DLLs and
    // per-user TSF choice. Never traverse SQLite, recordings or Credential Manager.
    eprintln!("Sayweave: shared input component retained; user data and enable choice retained.");
    remove_backups(&root, &record);
    let _ = fs::remove_dir(directory);
    Ok(())
}
fn verify_ready(record: &UserRecord) -> Result<()> {
    let machine = machine::load()?.ok_or("installerProofMissing")?;
    if machine.transaction != record.transaction
        || machine.phase != Phase::Committed
        || machine.target != record.target
        || machine::deployment_state(crate::text_service::payload()) != "ready"
        || hash(&read(
            &PathBuf::from(&record.install_dir).join("sayweave.exe"),
            MAX_FILE,
        )?) != record.target.main_hash
        || !record.user_initialized
        || registration::installer_user_choice(record.language)
            .map_err(|_| "componentUserChoice")?
            != Some(record.user_choice.unwrap_or(true))
    {
        return Err("installerProofIncomplete".into());
    }
    Ok(())
}
pub fn acknowledge_running(installer_hash: &str) -> Result<bool> {
    let exe = std::env::current_exe().map_err(|_| "installerExecutable")?;
    let directory = exe.parent().ok_or("installerDirectory")?;
    let root = record_dir(directory)?;
    let Some(record) = load(&root)? else {
        return Ok(false);
    };
    if record.target != running_target()? || record.installer.image_hash != installer_hash {
        return Ok(false);
    }
    verify_ready(&record)?;
    remove_backups(&root, &record);
    Ok(true)
}
fn remove_backups(root: &Path, record: &UserRecord) {
    // Backups are not part of user data. Remove only named, verified transaction files.
    for file in &record.files {
        let backup = root.join(&record.transaction).join(&file.backup);
        if backup.exists()
            && file.before_hash.as_ref().is_some_and(|expected| {
                read(&backup, MAX_FILE).is_ok_and(|bytes| hash(&bytes) == *expected)
            })
        {
            let _ = fs::remove_file(backup);
        }
    }
    let _ = fs::remove_dir(root.join(&record.transaction));
}
fn version(value: &str) -> Option<Vec<u32>> {
    value
        .split('.')
        .map(str::parse)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()
}
