#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    std::process::exit(sayweave_lib::installer_helper_entry().unwrap_or(2));
}
