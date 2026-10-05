fn main() {
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
    std::fs::write(out.join("text-service.dll"), payload).expect("copy build payload");
    tauri_build::build()
}
