//! Phase 2 — nhân bản lên cloud qua rclone (sidecar). Mỗi cloud là 1 replica
//! 1 chiều của bản working (SOT). File ở dạng thường trên Drive (xem/share trực tiếp).
//!
//! rclone tự lo OAuth + resumable upload + backoff. Module này chỉ: định vị
//! binary, liệt kê remote, và chạy copy/sync cho từng target (fan-out).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;

/// Bọc binary rclone.
#[derive(Clone)]
pub struct Rclone {
    bin: PathBuf,
    config: Option<PathBuf>,
}

/// Một đích cloud. `remote` là tên remote rclone (vd "gdrive:"), `dest_path`
/// là thư mục con trong remote đó (vd "backup/working").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudTarget {
    pub id: String,
    pub name: String,
    pub remote: String,
    pub dest_path: String,
    /// true = mirror (sync, xoá file thừa ở cloud theo SOT); false = chỉ thêm/cập nhật.
    #[serde(default)]
    pub mirror: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}
fn default_true() -> bool {
    true
}

impl CloudTarget {
    /// Chuỗi đích rclone: "remote:dest_path". Nếu remote rỗng thì coi dest_path
    /// là đường dẫn local (dùng cho test).
    pub fn dest(&self) -> String {
        if self.remote.is_empty() {
            self.dest_path.clone()
        } else {
            format!("{}{}", self.remote, self.dest_path)
        }
    }
}

#[derive(Debug, Clone)]
pub struct CopyOutcome {
    pub success: bool,
    pub output: String,
}

#[derive(Debug, Clone)]
pub struct TargetResult {
    pub target_id: String,
    pub target_name: String,
    pub outcome: Result<CopyOutcome, String>,
}

impl Rclone {
    /// Định vị rclone. Kiểm cả đường dẫn tuyệt đối phổ biến vì app GUI trên
    /// macOS/Linux khi mở từ Finder KHÔNG kế thừa PATH của shell.
    pub fn locate() -> Option<Rclone> {
        let candidates = [
            std::env::var("RCLONE_BIN").ok(),
            Some("rclone".to_string()),
            Some("/opt/homebrew/bin/rclone".to_string()),
            Some("/usr/local/bin/rclone".to_string()),
            Some("/usr/bin/rclone".to_string()),
        ];
        for c in candidates.into_iter().flatten() {
            let rc = Rclone {
                bin: PathBuf::from(&c),
                config: None,
            };
            if rc.version().is_ok() {
                return Some(rc);
            }
        }
        None
    }

    pub fn with_bin(bin: PathBuf) -> Rclone {
        Rclone { bin, config: None }
    }

    pub fn set_config(&mut self, path: PathBuf) {
        self.config = Some(path);
    }

    fn base_cmd(&self) -> Command {
        let mut cmd = Command::new(&self.bin);
        if let Some(cfg) = &self.config {
            cmd.arg("--config").arg(cfg);
        }
        cmd
    }

    pub fn version(&self) -> std::io::Result<String> {
        let out = self.base_cmd().arg("version").output()?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").to_string())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "rclone version lỗi",
            ))
        }
    }

    /// Tạo remote mới (kích hoạt OAuth: rclone tự mở trình duyệt cho user đồng ý).
    /// provider: "drive" (Google Drive), "onedrive" (OneDrive)...
    /// Lệnh này BLOCK tới khi user hoàn tất đăng nhập trong trình duyệt.
    pub fn config_create(&self, name: &str, provider: &str) -> std::io::Result<CopyOutcome> {
        let out = self
            .base_cmd()
            .arg("config")
            .arg("create")
            .arg(name)
            .arg(provider)
            .output()?;
        let mut combined = String::from_utf8_lossy(&out.stdout).to_string();
        combined.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(CopyOutcome {
            success: out.status.success(),
            output: combined.trim().to_string(),
        })
    }

    /// Liệt kê các remote đã cấu hình (vd ["gdrive:", "onedrive:"]).
    pub fn list_remotes(&self) -> std::io::Result<Vec<String>> {
        let out = self.base_cmd().arg("listremotes").output()?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }

    /// Đẩy toàn bộ `src` sang `dest`. mirror=true -> `sync` (xoá thừa), ngược lại `copy`.
    pub fn push(&self, src: &Path, dest: &str, mirror: bool) -> std::io::Result<CopyOutcome> {
        let sub = if mirror { "sync" } else { "copy" };
        let out = self
            .base_cmd()
            .arg(sub)
            .arg(src)
            .arg(dest)
            .arg("--transfers")
            .arg("4")
            .arg("--checkers")
            .arg("8")
            .arg("--stats-one-line")
            .arg("-v")
            .output()?;
        let mut combined = String::from_utf8_lossy(&out.stdout).to_string();
        combined.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(CopyOutcome {
            success: out.status.success(),
            output: combined.trim().to_string(),
        })
    }
}

/// Fan-out: đẩy `working_root` (SOT) tới tất cả target đang bật, mỗi target
/// chạy trên một luồng riêng (độc lập — target chậm/hỏng không chặn target khác).
pub fn replicate_all(rc: &Rclone, working_root: &Path, targets: &[CloudTarget]) -> Vec<TargetResult> {
    let (tx, rx) = mpsc::channel();
    let mut n = 0;
    for t in targets.iter().filter(|t| t.enabled) {
        n += 1;
        let rc = rc.clone();
        let t = t.clone();
        let root = working_root.to_path_buf();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let outcome = rc
                .push(&root, &t.dest(), t.mirror)
                .map_err(|e| e.to_string());
            let _ = tx.send(TargetResult {
                target_id: t.id.clone(),
                target_name: t.name.clone(),
                outcome,
            });
        });
    }
    drop(tx);
    let mut results = Vec::with_capacity(n);
    for _ in 0..n {
        if let Ok(r) = rx.recv() {
            results.push(r);
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bh_cloud_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn locate_and_replicate_to_local_remotes() {
        let rc = match Rclone::locate() {
            Some(rc) => rc,
            None => {
                eprintln!("BỎ QUA: rclone chưa cài — test tích hợp cloud không chạy");
                return;
            }
        };

        // nguồn (đóng vai working/SOT)
        let src = tmp("src");
        std::fs::write(src.join("a.txt"), "client-file-1").unwrap();
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/b.txt"), "client-file-2").unwrap();

        // 2 "cloud" giả lập bằng thư mục local (remote rỗng -> local path)
        let c1 = tmp("cloud1");
        let c2 = tmp("cloud2");
        let targets = vec![
            CloudTarget {
                id: "t1".into(),
                name: "Cloud A".into(),
                remote: String::new(),
                dest_path: c1.to_string_lossy().to_string(),
                mirror: false,
                enabled: true,
            },
            CloudTarget {
                id: "t2".into(),
                name: "Cloud B".into(),
                remote: String::new(),
                dest_path: c2.to_string_lossy().to_string(),
                mirror: false,
                enabled: true,
            },
        ];

        let results = replicate_all(&rc, &src, &targets);
        assert_eq!(results.len(), 2, "phải có kết quả cho cả 2 target");
        for r in &results {
            assert!(r.outcome.is_ok(), "target {} lỗi: {:?}", r.target_name, r.outcome);
            assert!(r.outcome.as_ref().unwrap().success);
        }
        // File phải xuất hiện ở cả 2 cloud.
        assert_eq!(std::fs::read_to_string(c1.join("a.txt")).unwrap(), "client-file-1");
        assert_eq!(std::fs::read_to_string(c1.join("sub/b.txt")).unwrap(), "client-file-2");
        assert_eq!(std::fs::read_to_string(c2.join("a.txt")).unwrap(), "client-file-1");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&c1);
        let _ = std::fs::remove_dir_all(&c2);
    }
}
