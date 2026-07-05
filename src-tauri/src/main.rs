// Ẩn cửa sổ console trên Windows khi chạy bản release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    backup_helper_lib::run()
}
