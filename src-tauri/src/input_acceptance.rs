fn main() {
    if std::env::var("SAYWEAVE_INPUT_ACCEPTANCE").as_deref() != Ok("1") {
        return;
    }
    let fixture = std::env::current_exe()
        .unwrap()
        .with_file_name("input-fixture.exe");
    match sayweave_lib::input_acceptance(&fixture) {
        Ok(()) => println!("SAYWEAVE_INPUT_ACCEPTANCE_PASS"),
        Err(error) => {
            eprintln!("SAYWEAVE_INPUT_ACCEPTANCE_FAIL {error}");
            std::process::exit(1);
        }
    }
}
