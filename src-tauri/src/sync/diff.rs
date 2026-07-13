//! Diff 3 phía: so origin & working với baseline để sinh kế hoạch đồng bộ.
//! Đây là "bộ não" phát hiện thay đổi + phân loại xung đột (ma trận ở §6).

use super::types::*;
use std::collections::BTreeSet;

/// Phân loại thay đổi của một phía (giá trị hiện tại `cur`) so với baseline `base`.
fn classify(cur: Option<&String>, base: Option<&String>) -> Change {
    match (cur, base) {
        (None, None) => Change::None,
        (Some(_), None) => Change::Created,
        (None, Some(_)) => Change::Deleted,
        (Some(c), Some(b)) => {
            if c == b {
                Change::None
            } else {
                Change::Modified
            }
        }
    }
}

/// So sánh 3 phía và trả về kế hoạch (thao tác an toàn + xung đột).
pub fn three_way(origin: &HashIndex, working: &HashIndex, baseline: &HashIndex) -> Plan {
    let mut plan = Plan::default();

    let mut paths: BTreeSet<&String> = BTreeSet::new();
    paths.extend(origin.keys());
    paths.extend(working.keys());
    paths.extend(baseline.keys());

    for path in paths {
        let o = origin.get(path);
        let w = working.get(path);
        let b = baseline.get(path);
        let oc = classify(o, b);
        let wc = classify(w, b);

        use Change::*;
        match (oc, wc) {
            // Không bên nào đổi.
            (None, None) => {}

            // Chỉ origin đổi -> đẩy sang working.
            (_, None) => match oc {
                Deleted => plan.ops.push(PlannedOp {
                    rel_path: path.clone(),
                    kind: OpKind::Delete(Side::Working),
                }),
                Created | Modified => plan.ops.push(PlannedOp {
                    rel_path: path.clone(),
                    kind: OpKind::Copy(Direction::OriginToWorking),
                }),
                None => {}
            },

            // Chỉ working đổi -> đẩy sang origin (reverse-sync).
            (None, _) => match wc {
                Deleted => plan.ops.push(PlannedOp {
                    rel_path: path.clone(),
                    kind: OpKind::Delete(Side::Origin),
                }),
                Created | Modified => plan.ops.push(PlannedOp {
                    rel_path: path.clone(),
                    kind: OpKind::Copy(Direction::WorkingToOrigin),
                }),
                None => {}
            },

            // Cả hai phía cùng đổi.
            _ => {
                if o == w {
                    // Hội tụ: cả 2 thành cùng trạng thái (cùng xóa, hoặc cùng nội dung).
                    // Không cần thao tác; baseline sẽ được cập nhật ở bước apply.
                } else if o.is_none() {
                    // origin xóa, working sửa.
                    plan.conflicts.push(Conflict {
                        rel_path: path.clone(),
                        kind: ConflictKind::EditVsDelete {
                            deleted: Side::Origin,
                        },
                    });
                } else if w.is_none() {
                    // working xóa, origin sửa.
                    plan.conflicts.push(Conflict {
                        rel_path: path.clone(),
                        kind: ConflictKind::EditVsDelete {
                            deleted: Side::Working,
                        },
                    });
                } else if b.is_none() {
                    // cả 2 cùng tạo mới, nội dung khác.
                    plan.conflicts.push(Conflict {
                        rel_path: path.clone(),
                        kind: ConflictKind::BothCreated,
                    });
                } else {
                    // cả 2 cùng sửa, nội dung khác.
                    plan.conflicts.push(Conflict {
                        rel_path: path.clone(),
                        kind: ConflictKind::BothModified,
                    });
                }
            }
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx(pairs: &[(&str, &str)]) -> HashIndex {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn op(plan: &Plan, path: &str) -> Option<OpKind> {
        plan.ops
            .iter()
            .find(|o| o.rel_path == path)
            .map(|o| o.kind.clone())
    }
    fn conf(plan: &Plan, path: &str) -> Option<ConflictKind> {
        plan.conflicts
            .iter()
            .find(|c| c.rel_path == path)
            .map(|c| c.kind.clone())
    }

    #[test]
    fn only_origin_modified_propagates_forward() {
        let base = idx(&[("a", "h1")]);
        let origin = idx(&[("a", "h2")]);
        let working = idx(&[("a", "h1")]);
        let p = three_way(&origin, &working, &base);
        assert_eq!(op(&p, "a"), Some(OpKind::Copy(Direction::OriginToWorking)));
        assert!(p.conflicts.is_empty());
    }

    #[test]
    fn only_working_modified_reverse_syncs() {
        let base = idx(&[("a", "h1")]);
        let origin = idx(&[("a", "h1")]);
        let working = idx(&[("a", "h2")]);
        let p = three_way(&origin, &working, &base);
        assert_eq!(op(&p, "a"), Some(OpKind::Copy(Direction::WorkingToOrigin)));
    }

    #[test]
    fn both_modified_same_content_converges_no_op() {
        let base = idx(&[("a", "h1")]);
        let origin = idx(&[("a", "h2")]);
        let working = idx(&[("a", "h2")]);
        let p = three_way(&origin, &working, &base);
        assert!(p.is_empty(), "cùng nội dung -> hội tụ, không thao tác");
    }

    #[test]
    fn both_modified_diff_content_is_conflict() {
        let base = idx(&[("a", "h1")]);
        let origin = idx(&[("a", "h2")]);
        let working = idx(&[("a", "h3")]);
        let p = three_way(&origin, &working, &base);
        assert_eq!(conf(&p, "a"), Some(ConflictKind::BothModified));
        assert!(p.ops.is_empty());
    }

    #[test]
    fn created_only_on_working_propagates_back() {
        let base = idx(&[]);
        let origin = idx(&[]);
        let working = idx(&[("new", "h1")]);
        let p = three_way(&origin, &working, &base);
        assert_eq!(op(&p, "new"), Some(OpKind::Copy(Direction::WorkingToOrigin)));
    }

    #[test]
    fn both_created_diff_is_conflict() {
        let base = idx(&[]);
        let origin = idx(&[("x", "h1")]);
        let working = idx(&[("x", "h2")]);
        let p = three_way(&origin, &working, &base);
        assert_eq!(conf(&p, "x"), Some(ConflictKind::BothCreated));
    }

    #[test]
    fn deleted_on_origin_propagates_delete() {
        let base = idx(&[("a", "h1")]);
        let origin = idx(&[]);
        let working = idx(&[("a", "h1")]);
        let p = three_way(&origin, &working, &base);
        assert_eq!(op(&p, "a"), Some(OpKind::Delete(Side::Working)));
    }

    #[test]
    fn edit_vs_delete_is_conflict() {
        let base = idx(&[("a", "h1")]);
        let origin = idx(&[]); // origin xóa
        let working = idx(&[("a", "h2")]); // working sửa
        let p = three_way(&origin, &working, &base);
        assert_eq!(
            conf(&p, "a"),
            Some(ConflictKind::EditVsDelete { deleted: Side::Origin })
        );
    }

    #[test]
    fn both_deleted_converges() {
        let base = idx(&[("a", "h1")]);
        let origin = idx(&[]);
        let working = idx(&[]);
        let p = three_way(&origin, &working, &base);
        assert!(p.is_empty());
    }
}
