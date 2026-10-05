fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let file =
            std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("service.def");
        println!("cargo:rustc-cdylib-link-arg=/DEF:{}", file.display());
        println!("cargo:rerun-if-changed=service.def");
    }
}
