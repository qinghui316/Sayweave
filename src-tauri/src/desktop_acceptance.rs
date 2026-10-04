#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Isolated acceptance executable; ordinary builds keep the product executable separate.
fn main() {
    sayweave_lib::run();
}
