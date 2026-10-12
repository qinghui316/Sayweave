fn main() {
    for name in [
        "SAYWEAVE_BUILD_CHANNEL",
        "SAYWEAVE_BUILD_COMMIT",
        "SAYWEAVE_TEST_UPDATE_URL",
        "SAYWEAVE_TEST_UPDATE_KEY",
        "SAYWEAVE_BUILD_VERSION",
        "SAYWEAVE_BUILD_IDENTIFIER",
        "SAYWEAVE_RELEASE_TAG",
        "TAURI_CONFIG",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let channel = std::env::var("SAYWEAVE_BUILD_CHANNEL").unwrap_or_else(|_| "internal".into());
    assert!(
        ["internal", "stable", "test"].contains(&channel.as_str()),
        "invalid build channel"
    );
    let git = |args: &[&str]| {
        let result = std::process::Command::new("git")
            .args(args)
            .output()
            .expect("git build identity");
        assert!(result.status.success(), "git build identity failed");
        String::from_utf8(result.stdout)
            .expect("git UTF-8")
            .trim()
            .to_owned()
    };
    let commit =
        std::env::var("SAYWEAVE_BUILD_COMMIT").unwrap_or_else(|_| git(&["rev-parse", "HEAD"]));
    if std::path::Path::new("../.git").exists() {
        let git_dir = git(&["rev-parse", "--absolute-git-dir"]);
        for file in ["HEAD", "packed-refs"] {
            println!("cargo:rerun-if-changed={git_dir}/{file}");
        }
        let reference = git(&["rev-parse", "--symbolic-full-name", "HEAD"]);
        if reference.starts_with("refs/") {
            println!("cargo:rerun-if-changed={git_dir}/{reference}");
        }
    }
    if channel == "stable" {
        assert!(
            std::env::var_os("CARGO_FEATURE_DESKTOP_ACCEPTANCE").is_none(),
            "acceptance feature in stable build"
        );
        assert_eq!(
            commit,
            git(&["rev-parse", "HEAD"]),
            "stable commit mismatch"
        );
        assert_eq!(commit.len(), 40);
        assert!(commit.bytes().all(|c| c.is_ascii_hexdigit()));
        assert!(
            git(&["status", "--porcelain", "--untracked-files=no"]).is_empty(),
            "dirty stable build"
        );
        let version = std::env::var("CARGO_PKG_VERSION").expect("version");
        let tag = std::env::var("SAYWEAVE_RELEASE_TAG").expect("stable release tag");
        assert_eq!(tag, format!("v{version}"));
        assert_eq!(git(&["rev-parse", &format!("{tag}^{{commit}}")]), commit);
        git(&["merge-base", "--is-ancestor", &commit, "origin/main"]);
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read("tauri.conf.json").expect("Tauri config"))
                .expect("Tauri config JSON");
        assert_eq!(config["version"], version);
        assert_eq!(config["identifier"], "local.sayweave.skeleton");
        assert_eq!(config["productName"], "Sayweave");
        assert_eq!(config["plugins"]["updater"]["requireSignedVersion"], true);
        if let Ok(overlay) = std::env::var("TAURI_CONFIG") {
            let overlay: serde_json::Value = serde_json::from_str(&overlay).expect("Tauri overlay");
            for key in ["version", "identifier", "productName", "plugins"] {
                if let Some(value) = overlay.get(key) {
                    assert_eq!(value, &config[key], "stable configuration override");
                }
            }
        }
    }
    if channel == "test" {
        assert!(
            std::env::var_os("CARGO_FEATURE_DESKTOP_ACCEPTANCE").is_some(),
            "test trust requires acceptance feature"
        );
        assert!(std::env::var("SAYWEAVE_TEST_UPDATE_URL").is_ok_and(|v| v.starts_with("https://")));
        assert!(std::env::var("SAYWEAVE_TEST_UPDATE_KEY").is_ok());
        assert!(
            std::env::var("SAYWEAVE_BUILD_IDENTIFIER")
                .is_ok_and(|id| id.starts_with("local.sayweave.acceptance.")),
            "test application identity required"
        );
    } else {
        assert!(
            std::env::var_os("SAYWEAVE_TEST_UPDATE_URL").is_none()
                && std::env::var_os("SAYWEAVE_TEST_UPDATE_KEY").is_none(),
            "test trust in non-test build"
        );
    }
    println!("cargo:rustc-env=SAYWEAVE_BUILD_CHANNEL={channel}");
    let effective_version = std::env::var("SAYWEAVE_BUILD_VERSION")
        .unwrap_or_else(|_| std::env::var("CARGO_PKG_VERSION").expect("version"));
    if channel != "test" {
        assert_eq!(
            effective_version,
            std::env::var("CARGO_PKG_VERSION").expect("version")
        );
    }
    println!("cargo:rustc-env=SAYWEAVE_BUILD_VERSION={effective_version}");
    let identifier = std::env::var("SAYWEAVE_BUILD_IDENTIFIER")
        .unwrap_or_else(|_| "local.sayweave.skeleton".into());
    if channel != "test" {
        assert_eq!(identifier, "local.sayweave.skeleton");
    }
    println!("cargo:rustc-env=SAYWEAVE_BUILD_IDENTIFIER={identifier}");
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read("tauri.conf.json").expect("Tauri config"))
            .expect("Tauri JSON");
    let overlay = std::env::var("TAURI_CONFIG")
        .ok()
        .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok());
    let product = overlay
        .as_ref()
        .and_then(|v| v["productName"].as_str())
        .or_else(|| config["productName"].as_str())
        .expect("product identity");
    assert!(
        !product.is_empty()
            && product.len() <= 128
            && !product.contains(['\\', '/', '\0', '\n', '\r']),
        "invalid product identity"
    );
    if channel != "test" {
        assert_eq!(product, "Sayweave");
    }
    println!("cargo:rustc-env=SAYWEAVE_BUILD_PRODUCT={product}");
    println!("cargo:rustc-env=SAYWEAVE_BUILD_COMMIT={commit}");
    println!(
        "cargo:rustc-env=SAYWEAVE_BUILD_TARGET={}",
        std::env::var("TARGET").expect("target")
    );
    println!("cargo:rerun-if-env-changed=SAYWEAVE_TSF_PAYLOAD");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let payload = if let Some(path) = std::env::var_os("SAYWEAVE_TSF_PAYLOAD") {
        println!(
            "cargo:rerun-if-changed={}",
            std::path::Path::new(&path).display()
        );
        std::fs::read(path).expect("read compiled TSF payload")
    } else {
        Vec::new()
    };
    assert!(
        payload.is_empty() || (payload.starts_with(b"MZ") && payload.len() <= 32 * 1024 * 1024),
        "invalid TSF payload"
    );
    std::fs::write(out.join("text-service-core.dll"), payload).expect("copy core payload");
    println!("cargo:rerun-if-env-changed=SAYWEAVE_TSF_PROXY_PAYLOAD");
    let proxy = if let Some(path) = std::env::var_os("SAYWEAVE_TSF_PROXY_PAYLOAD") {
        println!(
            "cargo:rerun-if-changed={}",
            std::path::Path::new(&path).display()
        );
        std::fs::read(path).expect("read fixed TSF proxy")
    } else {
        Vec::new()
    };
    assert!(
        proxy.is_empty() || (proxy.starts_with(b"MZ") && proxy.len() <= 32 * 1024 * 1024),
        "invalid TSF proxy"
    );
    std::fs::write(out.join("text-service.dll"), proxy).expect("copy proxy payload");
    tauri_build::build()
}
