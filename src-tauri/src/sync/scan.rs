//! Quét một thư mục thành chỉ mục, dùng fast-path stat để tránh hash lại file lớn.

use super::types::{Meta, MetaIndex};
use std::io;
use std::path::Path;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

/// Hash nội dung file bằng BLAKE3 (nhanh, song song với file lớn).
pub fn hash_file(path: &Path) -> io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut f = std::fs::File::open(path)?;
    io::copy(&mut f, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// mtime theo NANOSECOND (không phải giây) để không bỏ sót thay đổi cùng-size
/// xảy ra trong cùng một giây. Là fast-path; scrub định kỳ (băm lại toàn bộ)
/// là lưới an toàn cho các filesystem có độ phân giải mtime thô (FAT = 2s).
fn mtime_nanos(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Quét `root` thành MetaIndex. Nếu `prev` có entry cùng path với size+mtime
/// không đổi thì tái dùng hash cũ (không đọc lại file — quan trọng với file lớn).
pub fn scan_dir(root: &Path, prev: &MetaIndex) -> MetaIndex {
    let mut out = MetaIndex::new();
    if !root.is_dir() {
        return out;
    }
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let rel = match path.strip_prefix(root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = meta.len();
        let mtime = mtime_nanos(&meta);

        // Fast-path: stat khớp -> tái dùng hash cũ.
        if let Some(old) = prev.get(&rel) {
            if old.size == size && old.mtime == mtime {
                out.insert(rel, old.clone());
                continue;
            }
        }
        // Truth-path: hash lại nội dung.
        match hash_file(path) {
            Ok(hash) => {
                out.insert(rel, Meta { size, mtime, hash });
            }
            Err(_) => { /* file đang khóa/không đọc được -> bỏ qua lần này */ }
        }
    }
    out
}
