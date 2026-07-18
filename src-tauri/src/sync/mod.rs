//! Engine đồng bộ 2 chiều v2: metadata DB + phát hiện thay đổi +
//! diff 3 phía + version store để rollback.
//!
//! Bất biến an toàn: mọi file bị ghi đè/xóa đều được cất vào version store
//! TRƯỚC khi thay đổi; mọi thao tác được ghi journal để undo.

// Một số API (list_remotes, restore_version, has...) là bề mặt để nối UI ở
// Phase tiếp theo — cho phép chưa dùng tới trong lúc build backend.
#![allow(dead_code)]

pub mod db;
pub mod diff;
pub mod scan;
pub mod store;
pub mod types;

use db::{Db, JournalEntry, Version};
use std::io;
use std::path::{Path, PathBuf};
use types::*;

pub struct Engine {
    db: Db,
    store: store::Store,
}

/// Lỗi gộp cho engine.
#[derive(Debug)]
pub enum EngineError {
    Db(rusqlite::Error),
    Io(io::Error),
}
impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            EngineError::Db(e) => write!(f, "DB: {}", e),
            EngineError::Io(e) => write!(f, "IO: {}", e),
        }
    }
}
impl From<rusqlite::Error> for EngineError {
    fn from(e: rusqlite::Error) -> Self {
        EngineError::Db(e)
    }
}
impl From<io::Error> for EngineError {
    fn from(e: io::Error) -> Self {
        EngineError::Io(e)
    }
}
pub type Result<T> = std::result::Result<T, EngineError>;

fn now_micros() -> i64 {
    chrono::Local::now().timestamp_micros()
}

fn file_size(p: &Path) -> i64 {
    std::fs::metadata(p).map(|m| m.len() as i64).unwrap_or(0)
}

/// Copy nguyên tử: ghi ra temp cùng thư mục rồi rename.
fn atomic_copy(src: &Path, dst: &Path) -> io::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = dst.with_extension("synctmp");
    std::fs::copy(src, &tmp)?;
    // giữ mtime nguồn để lần quét sau nhận diện "không đổi".
    if let Ok(meta) = std::fs::metadata(src) {
        if let Ok(mt) = meta.modified() {
            let _ = filetime::set_file_mtime(&tmp, filetime::FileTime::from_system_time(mt));
        }
    }
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

impl Engine {
    pub fn open(db_path: &Path, store_dir: &Path) -> Result<Engine> {
        Ok(Engine {
            db: Db::open(db_path)?,
            store: store::Store::open(store_dir)?,
        })
    }

    #[cfg(test)]
    pub fn open_in_memory(store_dir: &Path) -> Result<Engine> {
        Ok(Engine {
            db: Db::open_in_memory()?,
            store: store::Store::open(store_dir)?,
        })
    }

    /// Quét 2 phía, lưu chỉ mục, so với baseline -> kế hoạch (read-only, an toàn).
    pub fn plan(&self, pair: &str, origin_root: &Path, working_root: &Path) -> Result<Plan> {
        let prev_o = self.db.get_index(pair, Side::Origin)?;
        let prev_w = self.db.get_index(pair, Side::Working)?;
        let cur_o = scan::scan_dir(origin_root, &prev_o);
        let cur_w = scan::scan_dir(working_root, &prev_w);
        self.db.set_index(pair, Side::Origin, &cur_o)?;
        self.db.set_index(pair, Side::Working, &cur_w)?;
        let base = self.db.get_baseline(pair)?;
        Ok(diff::three_way(
            &to_hash_index(&cur_o),
            &to_hash_index(&cur_w),
            &base,
        ))
    }

    /// Thi hành các thao tác an toàn của plan (conflict KHÔNG tự xử lý).
    /// Trả về run_id để undo.
    pub fn apply(
        &self,
        pair: &str,
        origin_root: &Path,
        working_root: &Path,
        plan: &Plan,
    ) -> Result<String> {
        let run_id = format!("run-{}", now_micros());
        let base = self.db.get_baseline(pair)?;

        for op in &plan.ops {
            let rel = &op.rel_path;
            let pre_base = base.get(rel).cloned();
            match &op.kind {
                OpKind::Copy(Direction::OriginToWorking) => {
                    self.do_copy(pair, rel, origin_root, working_root, Side::Working, &run_id, pre_base)?
                }
                OpKind::Copy(Direction::WorkingToOrigin) => {
                    self.do_copy(pair, rel, working_root, origin_root, Side::Origin, &run_id, pre_base)?
                }
                OpKind::Delete(Side::Working) => {
                    self.do_delete(pair, rel, working_root, Side::Working, &run_id, pre_base)?
                }
                OpKind::Delete(Side::Origin) => {
                    self.do_delete(pair, rel, origin_root, Side::Origin, &run_id, pre_base)?
                }
            }
        }

        // Sau khi áp dụng: quét lại, cập nhật chỉ mục, và tiến baseline cho
        // mọi path đã hội tụ (origin == working). Path còn conflict vẫn giữ nguyên.
        self.reconcile_baseline(pair, origin_root, working_root)?;
        Ok(run_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn do_copy(
        &self,
        pair: &str,
        rel: &str,
        src_root: &Path,
        dst_root: &Path,
        dst_side: Side,
        run_id: &str,
        pre_base: Option<String>,
    ) -> Result<()> {
        let src = src_root.join(rel);
        let dst = dst_root.join(rel);

        // Bất biến: cất bản cũ ở đích (nếu có) trước khi ghi đè.
        let pre_hash = if dst.exists() {
            let h = self.store.put(&dst)?;
            self.db
                .add_version(pair, rel, dst_side, &h, file_size(&dst), now_micros(), "overwrite")?;
            Some(h)
        } else {
            None
        };

        atomic_copy(&src, &dst)?;
        let post = self.store.put(&dst)?; // cache nội dung mới + lấy hash

        self.db.add_journal(&JournalEntry {
            run_id: run_id.to_string(),
            ts: now_micros(),
            kind: "copy".into(),
            pair: pair.to_string(),
            rel_path: rel.to_string(),
            abs_path: dst.to_string_lossy().to_string(),
            pre_hash,
            post_hash: Some(post),
            pre_base,
        })?;
        Ok(())
    }

    fn do_delete(
        &self,
        pair: &str,
        rel: &str,
        root: &Path,
        side: Side,
        run_id: &str,
        pre_base: Option<String>,
    ) -> Result<()> {
        let target = root.join(rel);
        if !target.exists() {
            return Ok(());
        }
        // Không xóa cứng: cất vào version store trước (đây là "thùng rác" có thể phục hồi).
        let h = self.store.put(&target)?;
        self.db
            .add_version(pair, rel, side, &h, file_size(&target), now_micros(), "delete")?;
        std::fs::remove_file(&target)?;

        self.db.add_journal(&JournalEntry {
            run_id: run_id.to_string(),
            ts: now_micros(),
            kind: "delete".into(),
            pair: pair.to_string(),
            rel_path: rel.to_string(),
            abs_path: target.to_string_lossy().to_string(),
            pre_hash: Some(h),
            post_hash: None,
            pre_base,
        })?;
        Ok(())
    }

    /// Quét lại 2 phía, cập nhật chỉ mục, tiến baseline cho path đã hội tụ.
    fn reconcile_baseline(&self, pair: &str, origin_root: &Path, working_root: &Path) -> Result<()> {
        let prev_o = self.db.get_index(pair, Side::Origin)?;
        let prev_w = self.db.get_index(pair, Side::Working)?;
        let cur_o = scan::scan_dir(origin_root, &prev_o);
        let cur_w = scan::scan_dir(working_root, &prev_w);
        self.db.set_index(pair, Side::Origin, &cur_o)?;
        self.db.set_index(pair, Side::Working, &cur_w)?;

        let ho = to_hash_index(&cur_o);
        let hw = to_hash_index(&cur_w);
        let base = self.db.get_baseline(pair)?;

        // Với mọi path xuất hiện ở 2 phía hoặc baseline:
        let mut all: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        all.extend(ho.keys().cloned());
        all.extend(hw.keys().cloned());
        all.extend(base.keys().cloned());

        for rel in all {
            match (ho.get(&rel), hw.get(&rel)) {
                (Some(a), Some(b)) if a == b => {
                    self.db.set_baseline_entry(pair, &rel, Some(a))?;
                }
                (None, None) => {
                    self.db.set_baseline_entry(pair, &rel, None)?;
                }
                _ => { /* còn khác nhau (conflict) -> giữ baseline cũ */ }
            }
        }
        Ok(())
    }

    /// Hoàn tác toàn bộ một lần sync: khôi phục nội dung file và baseline về trước.
    pub fn undo(&self, run_id: &str) -> Result<()> {
        let entries = self.db.journal_for_run(run_id)?; // đã theo thứ tự ngược
        for e in entries {
            let path = PathBuf::from(&e.abs_path);
            match e.kind.as_str() {
                "copy" => match &e.pre_hash {
                    Some(pre) => self.store.restore(pre, &path)?, // trả bản cũ về
                    None => {
                        let _ = std::fs::remove_file(&path); // trước đó không tồn tại -> xóa
                    }
                },
                "delete" => {
                    if let Some(pre) = &e.pre_hash {
                        self.store.restore(pre, &path)?; // phục hồi file đã xóa
                    }
                }
                _ => {}
            }
            self.db
                .set_baseline_entry(&e.pair, &e.rel_path, e.pre_base.as_deref())?;
        }
        Ok(())
    }

    /// Lịch sử phiên bản của một file.
    pub fn history(&self, pair: &str, rel: &str) -> Result<Vec<Version>> {
        Ok(self.db.list_versions(pair, rel)?)
    }

    /// Khôi phục một phiên bản cụ thể ra đường dẫn đích.
    pub fn restore_version(&self, version_id: i64, dst: &Path) -> Result<()> {
        match self.db.get_version(version_id)? {
            Some(v) => Ok(self.store.restore(&v.hash, dst)?),
            None => Err(EngineError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                "version không tồn tại",
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp {
        root: PathBuf,
    }
    impl Tmp {
        fn new(tag: &str) -> Tmp {
            let root = std::env::temp_dir().join(format!("bh_sync_{}_{}", std::process::id(), tag));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Tmp { root }
        }
        fn sub(&self, n: &str) -> PathBuf {
            let p = self.root.join(n);
            std::fs::create_dir_all(&p).unwrap();
            p
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    fn read(dir: &Path, rel: &str) -> String {
        std::fs::read_to_string(dir.join(rel)).unwrap()
    }

    fn engine(t: &Tmp) -> Engine {
        Engine::open_in_memory(&t.sub("store")).unwrap()
    }

    #[test]
    fn first_sync_seeds_baseline_no_ops() {
        let t = Tmp::new("seed");
        let (o, w) = (t.sub("origin"), t.sub("working"));
        write(&o, "a.txt", "hello");
        let eng = engine(&t);
        // Lần đầu: origin có file, working rỗng, baseline rỗng -> created bên origin.
        let plan = eng.plan("p", &o, &w).unwrap();
        assert_eq!(plan.ops.len(), 1);
        eng.apply("p", &o, &w, &plan).unwrap();
        assert_eq!(read(&w, "a.txt"), "hello");
        // Lần hai: đã hội tụ -> không còn thao tác.
        let plan2 = eng.plan("p", &o, &w).unwrap();
        assert!(plan2.is_empty(), "sau khi sync phải hội tụ");
    }

    #[test]
    fn reverse_sync_working_back_to_origin() {
        let t = Tmp::new("rev");
        let (o, w) = (t.sub("origin"), t.sub("working"));
        write(&o, "doc.txt", "v1");
        let eng = engine(&t);
        eng.apply("p", &o, &w, &eng.plan("p", &o, &w).unwrap()).unwrap();
        // Sửa ở bản working (làm việc với khách) -> phải đẩy ngược về origin.
        write(&w, "doc.txt", "v2-edited-with-client");
        let plan = eng.plan("p", &o, &w).unwrap();
        assert_eq!(plan.ops.len(), 1);
        eng.apply("p", &o, &w, &plan).unwrap();
        assert_eq!(read(&o, "doc.txt"), "v2-edited-with-client");
    }

    #[test]
    fn conflict_is_detected_and_not_applied() {
        let t = Tmp::new("conf");
        let (o, w) = (t.sub("origin"), t.sub("working"));
        write(&o, "f.txt", "base");
        let eng = engine(&t);
        eng.apply("p", &o, &w, &eng.plan("p", &o, &w).unwrap()).unwrap();
        // Cả 2 phía sửa khác nhau.
        write(&o, "f.txt", "origin-change");
        write(&w, "f.txt", "working-change");
        let plan = eng.plan("p", &o, &w).unwrap();
        assert_eq!(plan.conflicts.len(), 1);
        assert!(plan.ops.is_empty());
        eng.apply("p", &o, &w, &plan).unwrap();
        // Không bên nào bị ghi đè.
        assert_eq!(read(&o, "f.txt"), "origin-change");
        assert_eq!(read(&w, "f.txt"), "working-change");
    }

    #[test]
    fn undo_restores_overwritten_file_and_baseline() {
        let t = Tmp::new("undo");
        let (o, w) = (t.sub("origin"), t.sub("working"));
        write(&o, "x.txt", "v1");
        let eng = engine(&t);
        eng.apply("p", &o, &w, &eng.plan("p", &o, &w).unwrap()).unwrap();
        // working sửa -> sync ngược ghi đè origin.
        write(&w, "x.txt", "v2");
        let run = eng.apply("p", &o, &w, &eng.plan("p", &o, &w).unwrap()).unwrap();
        assert_eq!(read(&o, "x.txt"), "v2");
        // Undo -> origin trở lại v1.
        eng.undo(&run).unwrap();
        assert_eq!(read(&o, "x.txt"), "v1", "undo phải khôi phục nội dung cũ");
    }

    #[test]
    fn deleted_file_can_be_restored_from_history() {
        let t = Tmp::new("hist");
        let (o, w) = (t.sub("origin"), t.sub("working"));
        write(&o, "k.txt", "keepme");
        let eng = engine(&t);
        eng.apply("p", &o, &w, &eng.plan("p", &o, &w).unwrap()).unwrap();
        // Xóa ở origin -> propagate xóa sang working (có version trước khi xóa).
        std::fs::remove_file(o.join("k.txt")).unwrap();
        eng.apply("p", &o, &w, &eng.plan("p", &o, &w).unwrap()).unwrap();
        assert!(!w.join("k.txt").exists());
        // Lịch sử phải có bản đã xóa; khôi phục lại được.
        let versions = eng.history("p", "k.txt").unwrap();
        assert!(!versions.is_empty());
        let restored = t.root.join("restored.txt");
        eng.restore_version(versions[0].id, &restored).unwrap();
        assert_eq!(std::fs::read_to_string(&restored).unwrap(), "keepme");
    }
}
