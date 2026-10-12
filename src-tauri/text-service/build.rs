fn main() {
    println!("cargo:rerun-if-env-changed=SAYWEAVE_ENGINE_BUILD_ID");
    println!(
        "cargo:rustc-env=SAYWEAVE_ENGINE_BUILD_ID={}",
        std::env::var("SAYWEAVE_ENGINE_BUILD_ID").unwrap_or_else(|_| "0".into())
    );
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && (std::env::var_os("CARGO_FEATURE_COMPONENT").is_some()
            || std::env::var_os("CARGO_FEATURE_PROXY").is_some())
    {
        let mut exports = std::fs::read_to_string("service.def").expect("COM export declaration");
        if std::env::var_os("CARGO_FEATURE_PROXY").is_some() {
            if std::env::var_os("CARGO_FEATURE_ACCEPTANCE").is_some() {
                exports.push_str("    SayweaveProxyPrepareFixture PRIVATE\n    SayweaveProxyDispatchFixture PRIVATE\n    SayweaveProxyReadFixture PRIVATE\n    SayweaveProxyReadRetiredFixture PRIVATE\n    SayweaveProxyUseTransportFixture PRIVATE\n    SayweaveProxyActivationFaultFixture PRIVATE\n    SayweaveProxyInspectFixture PRIVATE\n    SayweaveProxyPinFixture PRIVATE\n    SayweaveProxyReleaseFixture PRIVATE\n");
            }
        } else if std::env::var_os("CARGO_FEATURE_ENGINE").is_some() {
            exports.push_str("    CreateEngineV1 PRIVATE\n");
        }
        let file = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("component.def");
        std::fs::write(&file, exports).expect("component exports");
        println!("cargo:rustc-cdylib-link-arg=/DEF:{}", file.display());
        println!("cargo:rerun-if-changed=service.def");
    }
}
