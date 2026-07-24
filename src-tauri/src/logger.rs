use chrono::{Local, NaiveDate};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// Ghi log ra file theo ngày và giữ lại đúng 2 ngày gần nhất (hôm nay + hôm qua)
/// để phục vụ audit. Mỗi dòng cũng được phát ra sự kiện "log" cho UI hiển thị.
pub struct Logger {
    dir: PathBuf,
    app: AppHandle,
}

impl Logger {
    pub fn new(app: &AppHandle) -> Self {
        let dir = app
            .path_resolver()
            .app_log_dir()
            .expect("không lấy được thư mục log");
        let _ = fs::create_dir_all(&dir);
        Logger {
            dir,
            app: app.clone(),
        }
    }

    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }

    pub fn log(&self, level: &str, msg: &str) {
        let now = Local::now();
        let line = format!(
            "[{}] {:<5} {}",
            now.format("%Y-%m-%d %H:%M:%S"),
            level,
            msg
        );
        let file = self.dir.join(format!("backup-{}.log", now.format("%Y-%m-%d")));
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&file) {
            let _ = writeln!(f, "{}", line);
        }
        let _ = self.app.emit_all("log", &line);
    }

    /// Xóa các file log cũ hơn 2 ngày (chỉ giữ hôm nay và hôm qua).
    pub fn cleanup(&self) {
        let today = Local::now().date_naive();
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(date_str) = name
                .strip_prefix("backup-")
                .and_then(|s| s.strip_suffix(".log"))
            {
                if let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                    let age = (today - date).num_days();
                    if age > 1 {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
    }

    /// Đọc toàn bộ log của 2 ngày gần nhất, theo thứ tự thời gian.
    pub fn read_recent(&self) -> Vec<String> {
        let today = Local::now().date_naive();
        let mut lines = Vec::new();
        for offset in [1i64, 0] {
            if let Some(date) = today.checked_sub_signed(chrono::Duration::days(offset)) {
                let file = self
                    .dir
                    .join(format!("backup-{}.log", date.format("%Y-%m-%d")));
                if let Ok(txt) = fs::read_to_string(&file) {
                    for l in txt.lines() {
                        lines.push(l.to_string());
                    }
                }
            }
        }
        lines
    }
}
