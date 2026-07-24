use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::AppHandle;

/// Một cặp thư mục cần sao lưu: nguồn -> đích.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Pair {
    pub id: String,
    pub source: String,
    pub dest: String,
    /// true = chế độ mirror (xóa ở đích nếu nguồn đã xóa).
    /// false = chỉ thêm/cập nhật, giữ lại file cũ ở đích.
    #[serde(default)]
    pub mirror: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Theo dõi thay đổi và sao lưu ngay lập tức.
    Realtime,
    /// Sao lưu định kỳ theo phút.
    Periodic,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Config {
    #[serde(default)]
    pub pairs: Vec<Pair>,
    #[serde(default = "default_mode")]
    pub mode: Mode,
    #[serde(default = "default_interval")]
    pub interval_minutes: u64,
    #[serde(default = "default_true")]
    pub autostart: bool,
    /// Có đang bật sao lưu tự động hay không (người dùng có thể tạm dừng).
    #[serde(default = "default_true")]
    pub running: bool,
}

fn default_mode() -> Mode {
    Mode::Periodic
}
fn default_interval() -> u64 {
    30
}

impl Default for Config {
    fn default() -> Self {
        Config {
            pairs: Vec::new(),
            mode: Mode::Periodic,
            interval_minutes: 30,
            autostart: true,
            running: true,
        }
    }
}

fn config_path(app: &AppHandle) -> PathBuf {
    let dir = app
        .path_resolver()
        .app_config_dir()
        .expect("không lấy được thư mục cấu hình");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("config.json")
}

pub fn load(app: &AppHandle) -> Config {
    let path = config_path(app);
    match std::fs::read_to_string(&path) {
        Ok(txt) => serde_json::from_str(&txt).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

pub fn save(app: &AppHandle, cfg: &Config) {
    let path = config_path(app);
    if let Ok(txt) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(path, txt);
    }
}
