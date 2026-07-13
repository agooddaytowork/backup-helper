//! Metadata DB (SQLite nhúng qua rusqlite). Lưu chỉ mục hash + baseline +
//! lịch sử phiên bản + nhật ký thao tác. Đây là "engine phát hiện thay đổi".

use super::types::{HashIndex, Meta, MetaIndex, Side};
use rusqlite::{params, Connection, Result};
use std::path::Path;

pub struct Db {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct Version {
    pub id: i64,
    pub pair: String,
    pub rel_path: String,
    pub side: String,
    pub hash: String,
    pub size: i64,
    pub created_at: i64,
    pub op: String,
}

#[derive(Debug, Clone)]
pub struct JournalEntry {
    pub run_id: String,
    pub ts: i64,
    pub kind: String,       // "copy" | "delete"
    pub pair: String,
    pub rel_path: String,
    pub abs_path: String,   // đường dẫn đích bị tác động
    pub pre_hash: Option<String>,  // nội dung trước khi thao tác (None = trước đó không tồn tại)
    pub post_hash: Option<String>, // nội dung sau (None = đã xóa)
    pub pre_base: Option<String>,  // baseline của path này TRƯỚC lần sync (để undo)
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let conn = Connection::open(path)?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Db> {
        let conn = Connection::open_in_memory()?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS file_index (
                pair TEXT NOT NULL, side TEXT NOT NULL, rel_path TEXT NOT NULL,
                size INTEGER NOT NULL, mtime INTEGER NOT NULL, hash TEXT NOT NULL,
                PRIMARY KEY (pair, side, rel_path)
            );
            CREATE TABLE IF NOT EXISTS baseline (
                pair TEXT NOT NULL, rel_path TEXT NOT NULL, hash TEXT NOT NULL,
                PRIMARY KEY (pair, rel_path)
            );
            CREATE TABLE IF NOT EXISTS versions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pair TEXT NOT NULL, rel_path TEXT NOT NULL, side TEXT NOT NULL,
                hash TEXT NOT NULL, size INTEGER NOT NULL,
                created_at INTEGER NOT NULL, op TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_versions_path ON versions(pair, rel_path);
            CREATE TABLE IF NOT EXISTS journal (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL, ts INTEGER NOT NULL, kind TEXT NOT NULL,
                pair TEXT NOT NULL, rel_path TEXT NOT NULL,
                abs_path TEXT NOT NULL, pre_hash TEXT, post_hash TEXT, pre_base TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_journal_run ON journal(run_id);
            "#,
        )
    }

    // ---------- chỉ mục ----------

    pub fn get_index(&self, pair: &str, side: Side) -> Result<MetaIndex> {
        let mut stmt = self.conn.prepare(
            "SELECT rel_path, size, mtime, hash FROM file_index WHERE pair=?1 AND side=?2",
        )?;
        let rows = stmt.query_map(params![pair, side.as_str()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Meta {
                    size: r.get::<_, i64>(1)? as u64,
                    mtime: r.get::<_, i64>(2)?,
                    hash: r.get::<_, String>(3)?,
                },
            ))
        })?;
        let mut idx = MetaIndex::new();
        for row in rows {
            let (k, v) = row?;
            idx.insert(k, v);
        }
        Ok(idx)
    }

    pub fn set_index(&self, pair: &str, side: Side, idx: &MetaIndex) -> Result<()> {
        let tx = &self.conn;
        tx.execute(
            "DELETE FROM file_index WHERE pair=?1 AND side=?2",
            params![pair, side.as_str()],
        )?;
        let mut stmt = tx.prepare(
            "INSERT INTO file_index(pair, side, rel_path, size, mtime, hash) VALUES (?1,?2,?3,?4,?5,?6)",
        )?;
        for (rel, m) in idx {
            stmt.execute(params![pair, side.as_str(), rel, m.size as i64, m.mtime, m.hash])?;
        }
        Ok(())
    }

    // ---------- baseline ----------

    pub fn get_baseline(&self, pair: &str) -> Result<HashIndex> {
        let mut stmt = self
            .conn
            .prepare("SELECT rel_path, hash FROM baseline WHERE pair=?1")?;
        let rows = stmt.query_map(params![pair], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut idx = HashIndex::new();
        for row in rows {
            let (k, v) = row?;
            idx.insert(k, v);
        }
        Ok(idx)
    }

    /// Upsert (hash Some) hoặc xóa (hash None) một entry baseline.
    pub fn set_baseline_entry(&self, pair: &str, rel: &str, hash: Option<&str>) -> Result<()> {
        match hash {
            Some(h) => self.conn.execute(
                "INSERT INTO baseline(pair, rel_path, hash) VALUES (?1,?2,?3)
                 ON CONFLICT(pair, rel_path) DO UPDATE SET hash=excluded.hash",
                params![pair, rel, h],
            )?,
            None => self.conn.execute(
                "DELETE FROM baseline WHERE pair=?1 AND rel_path=?2",
                params![pair, rel],
            )?,
        };
        Ok(())
    }

    // ---------- versions ----------

    #[allow(clippy::too_many_arguments)]
    pub fn add_version(
        &self,
        pair: &str,
        rel: &str,
        side: Side,
        hash: &str,
        size: i64,
        created_at: i64,
        op: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO versions(pair, rel_path, side, hash, size, created_at, op)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![pair, rel, side.as_str(), hash, size, created_at, op],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_versions(&self, pair: &str, rel: &str) -> Result<Vec<Version>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, pair, rel_path, side, hash, size, created_at, op
             FROM versions WHERE pair=?1 AND rel_path=?2 ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt.query_map(params![pair, rel], row_to_version)?;
        rows.collect()
    }

    pub fn get_version(&self, id: i64) -> Result<Option<Version>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, pair, rel_path, side, hash, size, created_at, op FROM versions WHERE id=?1",
        )?;
        let mut rows = stmt.query_map(params![id], row_to_version)?;
        match rows.next() {
            Some(v) => Ok(Some(v?)),
            None => Ok(None),
        }
    }

    // ---------- journal ----------

    pub fn add_journal(&self, e: &JournalEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO journal(run_id, ts, kind, pair, rel_path, abs_path, pre_hash, post_hash, pre_base)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                e.run_id, e.ts, e.kind, e.pair, e.rel_path, e.abs_path, e.pre_hash, e.post_hash,
                e.pre_base
            ],
        )?;
        Ok(())
    }

    /// Lấy các thao tác của một lần chạy, theo thứ tự NGƯỢC (để undo).
    pub fn journal_for_run(&self, run_id: &str) -> Result<Vec<JournalEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, ts, kind, pair, rel_path, abs_path, pre_hash, post_hash, pre_base
             FROM journal WHERE run_id=?1 ORDER BY id DESC",
        )?;
        let rows = stmt.query_map(params![run_id], |r| {
            Ok(JournalEntry {
                run_id: r.get(0)?,
                ts: r.get(1)?,
                kind: r.get(2)?,
                pair: r.get(3)?,
                rel_path: r.get(4)?,
                abs_path: r.get(5)?,
                pre_hash: r.get(6)?,
                post_hash: r.get(7)?,
                pre_base: r.get(8)?,
            })
        })?;
        rows.collect()
    }
}

fn row_to_version(r: &rusqlite::Row) -> Result<Version> {
    Ok(Version {
        id: r.get(0)?,
        pair: r.get(1)?,
        rel_path: r.get(2)?,
        side: r.get(3)?,
        hash: r.get(4)?,
        size: r.get(5)?,
        created_at: r.get(6)?,
        op: r.get(7)?,
    })
}
