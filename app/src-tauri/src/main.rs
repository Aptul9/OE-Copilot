// Niente finestra di console in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    oe_copilot_app_lib::run();
}
