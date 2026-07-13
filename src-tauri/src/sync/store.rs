//! Kho phiên bản content-addressed (CAS). Mỗi nội dung file lưu 1 lần theo hash.
//! Đây là nền cho rollback: trước khi ghi đè/xóa, bytes cũ luôn được cất vào đây.

use super::scan::hash_file;
use std::io;
use std::path::{Path, PathBuf};

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn open(root: &Path) -> io::Result<Store> {
        std::fs::create_dir_all(root)?;
        Ok(Store {
            root: root.to_path_buf(),
        })
    }

    fn blob_path(&self, hash: &str) -> PathBuf {
        // Chia thư mục theo 2 ký tự đầu để tránh 1 folder quá nhiều file.
        let (a, b) = hash.split_at(2.min(hash.len()));
        self.root.join(a).join(b)
    }

    /// Cất nội dung file vào kho (nếu chưa có). Trả về hash.
    pub fn put(&self, src: &Path) -> io::Result<String> {
        let hash = hash_file(src)?;
        let dst = self.blob_path(&hash);
        if dst.exists() {
            return Ok(hash); // dedup: đã có nội dung này
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // ghi ra temp rồi rename để nguyên tử.
        let tmp = dst.with_extension("tmp");
        std::fs::copy(src, &tmp)?;
        std::fs::rename(&tmp, &dst)?;
        Ok(hash)
    }

    pub fn has(&self, hash: &str) -> bool {
        self.blob_path(hash).exists()
    }

    /// Khôi phục nội dung theo hash ra `dst` (ghi nguyên tử).
    pub fn restore(&self, hash: &str, dst: &Path) -> io::Result<()> {
        let blob = self.blob_path(hash);
        if !blob.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("blob không tồn tại: {}", hash),
            ));
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = dst.with_extension("resttmp");
        std::fs::copy(&blob, &tmp)?;
        std::fs::rename(&tmp, dst)?;
        Ok(())
    }
}
