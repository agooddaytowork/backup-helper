//! Kiểu dữ liệu dùng chung cho engine đồng bộ 2 chiều v2.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Metadata một file trong chỉ mục (dùng cho fast-path stat + hash nội dung).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub size: u64,
    pub mtime: i64,
    pub hash: String,
}

/// Chỉ mục đầy đủ: rel_path -> Meta.
pub type MetaIndex = BTreeMap<String, Meta>;
/// Chỉ mục rút gọn cho diff: rel_path -> content hash.
pub type HashIndex = BTreeMap<String, String>;

pub fn to_hash_index(m: &MetaIndex) -> HashIndex {
    m.iter().map(|(k, v)| (k.clone(), v.hash.clone())).collect()
}

/// Phía tham gia đồng bộ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Origin,
    Working,
}

impl Side {
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Origin => "origin",
            Side::Working => "working",
        }
    }
}

/// Phân loại thay đổi của một phía so với baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    None,
    Created,
    Modified,
    Deleted,
}

/// Hướng copy khi propagate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    OriginToWorking,
    WorkingToOrigin,
}

/// Một thao tác an toàn (không phải conflict) trong kế hoạch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OpKind {
    Copy(Direction),
    Delete(Side),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedOp {
    pub rel_path: String,
    pub kind: OpKind,
}

/// Loại xung đột cần người dùng quyết định.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConflictKind {
    /// Cả 2 phía cùng sửa nội dung khác nhau.
    BothModified,
    /// Cả 2 phía cùng tạo mới nhưng nội dung khác nhau.
    BothCreated,
    /// Một phía xóa, phía kia sửa.
    EditVsDelete { deleted: Side },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub rel_path: String,
    pub kind: ConflictKind,
}

/// Kế hoạch đồng bộ: thao tác an toàn + danh sách xung đột.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub ops: Vec<PlannedOp>,
    pub conflicts: Vec<Conflict>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty() && self.conflicts.is_empty()
    }
}
