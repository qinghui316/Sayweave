use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn stage(file: &Path, profile: &Path, manifest: &mut BTreeMap<String, serde_json::Value>) {
    println!("cargo:rerun-if-changed={}", file.display());
    let bytes = fs::read(file).expect("runtime file");
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let name = file
        .file_name()
        .expect("runtime name")
        .to_str()
        .expect("UTF-8 runtime name");
    if let Some(previous) = manifest.get(name) {
        assert_eq!(previous["sha256"], digest, "conflicting runtime file");
        return;
    }
    let destination = profile.join(name);
    if !fs::read(&destination).is_ok_and(|existing| existing == bytes) {
        fs::write(&destination, &bytes)
            .unwrap_or_else(|e| panic!("stage {}: {e}", destination.display()));
    }
    manifest.insert(
        name.into(),
        serde_json::json!({"sha256":digest,"bytes":bytes.len()}),
    );
}

fn crt_directory() -> PathBuf {
    for key in ["SAYWEAVE_VC_REDIST_DIR", "VCToolsRedistDir"] {
        println!("cargo:rerun-if-env-changed={key}");
        if let Some(value) = env::var_os(key) {
            let root = PathBuf::from(value);
            for candidate in [root.clone(), root.join("x64/Microsoft.VC143.CRT")] {
                if candidate.join("msvcp140.dll").is_file() {
                    return candidate;
                }
            }
        }
    }
    let program_files = env::var_os("ProgramFiles(x86)").expect("ProgramFiles(x86)");
    let vswhere =
        PathBuf::from(program_files).join("Microsoft Visual Studio/Installer/vswhere.exe");
    let found = Command::new(vswhere)
        .args(["-latest", "-products", "*", "-property", "installationPath"])
        .output()
        .expect("vswhere; or set SAYWEAVE_VC_REDIST_DIR");
    assert!(found.status.success(), "vswhere failed");
    let install = String::from_utf8(found.stdout).expect("VS path encoding");
    let root = PathBuf::from(install.trim()).join("VC/Redist/MSVC");
    let mut versions = fs::read_dir(root)
        .expect("MSVC redist directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    versions.sort();
    versions
        .into_iter()
        .rev()
        .map(|version| version.join("x64/Microsoft.VC143.CRT"))
        .find(|p| p.join("msvcp140.dll").is_file())
        .expect("MSVC CRT; or set SAYWEAVE_VC_REDIST_DIR")
}

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_TRANSCRIBE_CPP_RUNTIME_DIR");
    println!("cargo:rerun-if-env-changed=DEP_TRANSCRIBE_CPP_MODULE_DIR");
    let Some(runtime) = env::var_os("DEP_TRANSCRIBE_CPP_RUNTIME_DIR") else {
        return;
    };
    let mut paths = vec![PathBuf::from(runtime)];
    if let Some(modules) = env::var_os("DEP_TRANSCRIBE_CPP_MODULE_DIR") {
        let modules = PathBuf::from(modules);
        if !paths.contains(&modules) {
            paths.push(modules);
        }
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let profile = out.ancestors().nth(3).expect("Cargo output shape");
    let mut manifest = BTreeMap::new();
    for path in paths {
        for entry in fs::read_dir(path).expect("native runtime directory") {
            let file = entry.expect("native runtime entry").path();
            if file
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
            {
                stage(&file, profile, &mut manifest);
            }
        }
    }
    assert!(!manifest.is_empty(), "dynamic inference runtime missing");
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let directory = crt_directory();
        for name in [
            "msvcp140.dll",
            "msvcp140_1.dll",
            "msvcp140_2.dll",
            "vcruntime140.dll",
            "vcruntime140_1.dll",
            "concrt140.dll",
        ] {
            stage(&directory.join(name), profile, &mut manifest);
        }
    }
    fs::write(profile.join("local-asr-runtime-manifest.json"),serde_json::to_vec_pretty(&serde_json::json!({"version":1,"engine":"0.1.3","target":env::var("TARGET").expect("TARGET"),"files":manifest})).expect("runtime manifest")).expect("stage runtime manifest");
}
