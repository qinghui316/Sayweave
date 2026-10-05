#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    if let Some(code) = sayweave_lib::text_service_helper_entry() {
        std::process::exit(code);
    }
    sayweave_lib::run();
}
