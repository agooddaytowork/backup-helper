use crate::config::Pair;
use crate::logger::Logger;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

/// Trừu tượng ghi log để engine dùng Logger thật còn test dùng bản giả.
pub trait SyncLog {
    fn log(&self, level: &str, msg: &str);
}

impl SyncLog for Logger {
    fn log(&self, level: &str, msg: &str) {
        Logger::log(self, level, msg);
    }
}

#[derive(Default, Debug, Clone)]
pub struct SyncResult {
    pub copied: u64,
    pub deleted: u64,
    pub skipped: u64,
    pub errors: u64,
    pub bytes: u64,
}

/// Kiểm tra file nguồn có cần copy sang đích không.
/// Copy khi: đích chưa có, khác kích thước, hoặc nguồn mới hơn đích (dung sai 2s).
fn need_copy(src: &Path, dst: &Path) -> bool {
    let src_meta = match fs::metadata(src) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let dst_meta = match fs::metadata(dst) {
        Ok(m) => m,
        Err(_) => return true, // đích chưa tồn tại
    };
    if src_meta.len() != dst_meta.len() {
        return true;
    }
    let src_t = mtime_secs(&src_meta);
    let dst_t = mtime_secs(&dst_meta);
    src_t > dst_t + 2
}

fn mtime_secs(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Sau khi copy, gán lại mtime của đích bằng nguồn để lần sau nhận diện "không đổi".
fn preserve_mtime(src: &Path, dst: &Path) {
    if let Ok(meta) = fs::metadata(src) {
        if let Ok(mt) = meta.modified() {
            let ft = filetime::FileTime::from_system_time(mt);
            let _ = filetime::set_file_mtime(dst, ft);
        }
    }
}

/// Đồng bộ một cặp nguồn -> đích. Chỉ copy file có thay đổi.
pub fn sync_pair(pair: &Pair, logger: &dyn SyncLog) -> SyncResult {
    let mut res = SyncResult::default();
    let source = Path::new(&pair.source);
    let dest = Path::new(&pair.dest);

    if !source.is_dir() {
        logger.log(
            "ERROR",
            &format!("Bỏ qua: thư mục nguồn không tồn tại: {}", pair.source),
        );
        res.errors += 1;
        return res;
    }
    if let Err(e) = fs::create_dir_all(dest) {
        logger.log(
            "ERROR",
            &format!("Không tạo được thư mục đích {}: {}", pair.dest, e),
        );
        res.errors += 1;
        return res;
    }

    // Tập các đường dẫn tương đối tồn tại ở nguồn (dùng cho chế độ mirror).
    let mut present: HashSet<std::path::PathBuf> = HashSet::new();

    for entry in WalkDir::new(source).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = match path.strip_prefix(source) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dest.join(rel);
        present.insert(rel.to_path_buf());

        if entry.file_type().is_dir() {
            if let Err(e) = fs::create_dir_all(&target) {
                logger.log("ERROR", &format!("Lỗi tạo thư mục {:?}: {}", target, e));
                res.errors += 1;
            }
            continue;
        }

        if !entry.file_type().is_file() {
            continue; // bỏ qua symlink/thiết bị đặc biệt
        }

        if need_copy(path, &target) {
            if let Some(parent) = target.parent() {
                let _ = fs::create_dir_all(parent);
            }
            match fs::copy(path, &target) {
                Ok(n) => {
                    preserve_mtime(path, &target);
                    res.copied += 1;
                    res.bytes += n;
                    logger.log("COPY", &format!("{}", rel.display()));
                }
                Err(e) => {
                    res.errors += 1;
                    logger.log("ERROR", &format!("Lỗi copy {}: {}", rel.display(), e));
                }
            }
        } else {
            res.skipped += 1;
        }
    }

    if pair.mirror {
        mirror_delete(dest, &present, logger, &mut res);
    }

    res
}

/// Xóa ở đích những file/thư mục không còn ở nguồn (chế độ mirror).
fn mirror_delete(
    dest: &Path,
    present: &HashSet<std::path::PathBuf>,
    logger: &dyn SyncLog,
    res: &mut SyncResult,
) {
    // Duyệt ngược (contents_first) để xóa file trước, thư mục sau.
    for entry in WalkDir::new(dest)
        .contents_first(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let rel = match entry.path().strip_prefix(dest) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if rel.as_os_str().is_empty() || present.contains(rel) {
            continue;
        }
        let p = entry.path();
        let result = if entry.file_type().is_dir() {
            fs::remove_dir_all(p)
        } else {
            fs::remove_file(p)
        };
        match result {
            Ok(_) => {
                res.deleted += 1;
                logger.log("DELETE", &format!("{}", rel.display()));
            }
            Err(e) => {
                res.errors += 1;
                logger.log("ERROR", &format!("Lỗi xóa {}: {}", rel.display(), e));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopLog;
    impl SyncLog for NoopLog {
        fn log(&self, _level: &str, _msg: &str) {}
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bh_test_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn pair(src: &Path, dst: &Path, mirror: bool) -> Pair {
        Pair {
            id: "t".into(),
            source: src.to_string_lossy().to_string(),
            dest: dst.to_string_lossy().to_string(),
            mirror,
            enabled: true,
        }
    }

    #[test]
    fn incremental_copy_skip_and_mirror() {
        let src = tmp("src");
        let dst = tmp("dst");
        fs::write(src.join("a.txt"), "hello").unwrap();
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/b.txt"), "world").unwrap();

        // Lần đầu: copy cả 2 file.
        let r = sync_pair(&pair(&src, &dst, false), &NoopLog);
        assert_eq!(r.copied, 2, "phải copy 2 file lần đầu");
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
        assert!(dst.join("sub/b.txt").exists());

        // Lần hai: không đổi -> bỏ qua hết.
        let r = sync_pair(&pair(&src, &dst, false), &NoopLog);
        assert_eq!(r.copied, 0, "không có thay đổi thì không copy");
        assert_eq!(r.skipped, 2);

        // Sửa 1 file (khác kích thước) -> copy đúng 1.
        fs::write(src.join("a.txt"), "hello-changed").unwrap();
        let r = sync_pair(&pair(&src, &dst, false), &NoopLog);
        assert_eq!(r.copied, 1, "chỉ copy file đã đổi");
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello-changed");

        // Xóa nguồn, chế độ giữ lại -> đích vẫn còn.
        fs::remove_file(src.join("a.txt")).unwrap();
        let r = sync_pair(&pair(&src, &dst, false), &NoopLog);
        assert_eq!(r.deleted, 0);
        assert!(dst.join("a.txt").exists(), "chế độ giữ lại không được xóa");

        // Chế độ mirror -> xóa theo nguồn.
        let r = sync_pair(&pair(&src, &dst, true), &NoopLog);
        assert_eq!(r.deleted, 1, "mirror phải xóa file thừa ở đích");
        assert!(!dst.join("a.txt").exists());
        assert!(dst.join("sub/b.txt").exists(), "file còn ở nguồn phải giữ");

        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn missing_source_reports_error() {
        let dst = tmp("dst2");
        let r = sync_pair(&pair(Path::new("/khong/ton/tai/xyz"), &dst, false), &NoopLog);
        assert_eq!(r.errors, 1);
        assert_eq!(r.copied, 0);
        let _ = fs::remove_dir_all(&dst);
    }
}
