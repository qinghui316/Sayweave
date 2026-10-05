fn main() {
    if std::env::var("SAYWEAVE_INPUT_ACCEPTANCE").as_deref() != Ok("1") {
        return;
    }
    struct Diagnostics;
    impl log::Log for Diagnostics {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.target() == "sayweave"
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                eprintln!("{}", record.args());
            }
        }
        fn flush(&self) {}
    }
    static DIAGNOSTICS: Diagnostics = Diagnostics;
    let _ = log::set_logger(&DIAGNOSTICS);
    log::set_max_level(log::LevelFilter::Info);
    let fixture = std::env::current_exe()
        .unwrap()
        .with_file_name("input-fixture.exe");
    let result = if std::env::args().nth(1).as_deref() == Some("composition") {
        sayweave_lib::composition_acceptance(&fixture)
    } else {
        sayweave_lib::input_acceptance(&fixture)
    };
    match result {
        Ok(()) => println!("SAYWEAVE_INPUT_ACCEPTANCE_PASS"),
        Err(error) => {
            eprintln!("SAYWEAVE_INPUT_ACCEPTANCE_FAIL {error}");
            std::process::exit(1);
        }
    }
}
