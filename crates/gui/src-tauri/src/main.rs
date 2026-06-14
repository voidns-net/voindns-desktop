// Prevents an additional console window on Windows in release; ignored on other platforms.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    voidns_gui_lib::run()
}
