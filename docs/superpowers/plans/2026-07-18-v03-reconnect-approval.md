# v0.3.0 — Reconnect + Duyệt sync, bỏ cloud — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Gỡ toàn bộ cloud (rclone); thêm watcher phát hiện mất/có lại kết nối thư mục với chốt an toàn chặn sync khi mất kết nối; luồng duyệt trước khi sync lúc reconnect; scheduler báo conflict; UI xem lịch sử phiên bản.

**Architecture:** Engine v2 (`sync/`) giữ nguyên trừ việc xóa `cloud.rs`. Mọi logic mới nằm trong `v2.rs` (watcher thread + guard + command mới) và `ui/v2.js`/`ui/index.html` (badge kết nối, thẻ duyệt, lịch sử). Giao tiếp Rust→UI qua Tauri event (`v2-conn-changed`, `v2-reconnected`, `v2-conflicts`).

**Tech Stack:** Rust (Tauri 2, rusqlite, blake3, walkdir), vanilla JS (`window.__TAURI__`), không framework FE.

**Spec:** `docs/superpowers/specs/2026-07-18-v03-reconnect-approval-design.md`

## Global Constraints

- Workflow v1 (`backup.rs`, `engine.rs`, `config.rs`, `ui/main.js`, tab `#tab-backup` trừ text nút Nâng cao) KHÔNG được đụng logic.
- Lõi engine v2 (`sync/mod.rs` trừ dòng `pub mod cloud`, `db.rs`, `diff.rs`, `scan.rs`, `store.rs`, `types.rs`) KHÔNG đổi.
- Config cũ có field `targets` phải vẫn parse được (serde mặc định bỏ qua field lạ — không thêm `deny_unknown_fields`).
- Toàn bộ text UI + comment code bằng tiếng Việt, khớp giọng văn hiện có.
- Poll kết nối: 5 giây, chỉ `read_dir` thư mục gốc mỗi phía; KHÔNG giữ lock `SyncManager` trong lúc gọi `read_dir` (ổ mạng chết có thể block hàng chục giây).
- Reconnect KHÔNG bao giờ tự apply — chỉ mở cửa sổ + hiện thẻ duyệt.
- Chạy test: `cargo test --manifest-path src-tauri/Cargo.toml` (từ repo root).
- Version đích: `0.3.0` (Cargo.toml, tauri.conf.json, package.json — hiện đang là 0.2.1).

---

### Task 1: Gỡ cloud phía Rust

**Files:**
- Delete: `src-tauri/src/sync/cloud.rs`
- Modify: `src-tauri/src/sync/mod.rs:11` (bỏ `pub mod cloud;`)
- Modify: `src-tauri/src/v2.rs` (bỏ import cloud, field `targets`/`rclone`, `replicate`, `ReplStatus`, `V2Status`, 5 command cloud)
- Modify: `src-tauri/src/lib.rs:334-349` (bỏ 5 command khỏi `generate_handler!`)

**Interfaces:**
- Consumes: —
- Produces: `SyncManager::new(engine: Engine, cfg_path: PathBuf) -> SyncManager` (mất tham số `rclone_cfg`); `ApplyReport { run_id: String, copied: usize, deleted: usize, conflicts: Vec<Conflict> }` (mất `replication`); `V2Config { pairs, last_run, auto, interval_minutes }` (mất `targets`). Task 4-5 sửa tiếp `v2.rs` dựa trên hình dạng này.

- [ ] **Step 1: Xóa file cloud.rs và module declaration**

```bash
git rm src-tauri/src/sync/cloud.rs
```

Trong `src-tauri/src/sync/mod.rs` xóa dòng:

```rust
pub mod cloud;
```

- [ ] **Step 2: Dọn v2.rs**

Các sửa đổi trong `src-tauri/src/v2.rs`:

1. Header comment + import — thay:

```rust
//! Lớp wiring v2: nối engine đồng bộ 2 chiều + replication rclone vào Tauri.
//! Quản lý cấu hình (cặp origin↔working + cloud target), điều phối
//! plan/apply/resolve/undo/history và fan-out cloud.

use crate::sync::cloud::{replicate_all, CloudTarget, Rclone};
```

bằng:

```rust
//! Lớp wiring v2: nối engine đồng bộ 2 chiều vào Tauri.
//! Quản lý cấu hình cặp origin↔working, điều phối plan/apply/resolve/undo/history.

```

2. `V2Config`: xóa field `#[serde(default)] pub targets: Vec<CloudTarget>,` và dòng `targets: vec![],` trong `impl Default`.

3. `SyncManager`: xóa field `rclone: Option<Rclone>,`. Sửa constructor thành:

```rust
    pub fn new(engine: Engine, cfg_path: PathBuf) -> SyncManager {
        let cfg = std::fs::read_to_string(&cfg_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        SyncManager { engine, cfg, cfg_path }
    }
```

4. Xóa toàn bộ: struct `V2Status`, struct `ReplStatus`, hàm `SyncManager::replicate`, field `replication` trong `ApplyReport`, và trong `SyncManager::apply` xóa 2 dòng `let replication = self.replicate(&p);` + `replication,`.

5. Xóa 5 command: `v2_status`, `v2_add_target`, `v2_remove_target`, `v2_replicate`, `v2_connect_remote` (cả comment `// ---------- cloud target ----------`).

6. `init()` cuối file — thay dòng tạo SyncManager bằng:

```rust
    SyncManager::new(engine, v2dir.join("v2-config.json"))
```

7. Trong `start_scheduler`, sửa comment `// apply đã gồm cả local + fan-out cloud` thành `// apply chỉ chạy thao tác an toàn; conflict để người dùng quyết`.

- [ ] **Step 3: Dọn lib.rs**

Trong `generate_handler![...]` xóa 5 dòng: `v2::v2_status,`, `v2::v2_add_target,`, `v2::v2_remove_target,`, `v2::v2_replicate,`, `v2::v2_connect_remote,`. Sửa comment `// State v2: engine đồng bộ 2 chiều + replication cloud.` thành `// State v2: engine đồng bộ 2 chiều.`

- [ ] **Step 4: Build + test phải sạch**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS toàn bộ (5 test sẵn có trong `sync/mod.rs`), không warning về symbol cloud còn sót.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "refactor(v2): gỡ toàn bộ cloud/rclone phía Rust"
```

---

### Task 2: Gỡ cloud phía UI

**Files:**
- Modify: `ui/index.html` (xóa section Cloud + Đích cloud + modal target; sửa text)
- Modify: `ui/v2.js` (xóa code cloud)

**Interfaces:**
- Consumes: các command còn lại sau Task 1.
- Produces: `renderPairs()` không còn nút "Đẩy cloud" — Task 6 sẽ sửa tiếp hàm này.

- [ ] **Step 1: Sửa index.html**

Xóa 2 section `<!-- Cloud -->` (dòng ~147-161) và `<!-- Đích cloud -->` (~163-171), xóa modal `<div id="v2TargetDlg">…</div>` (~196-218).

Sửa text:
- `title="Đồng bộ 2 chiều, cloud, hoàn tác"` → `title="Đồng bộ 2 chiều, hoàn tác, lịch sử"`
- `<div class="brand-title">Đồng bộ 2 chiều &amp; Cloud</div>` → `<div class="brand-title">Đồng bộ 2 chiều (Gốc ⇄ Bản làm việc)</div>`
- Header section Tự động: `<h2>Tự động đồng bộ &amp; đẩy cloud</h2>` → `<h2>Tự động đồng bộ định kỳ</h2>`
- Label checkbox tự động: `<span>Tự động chạy định kỳ (đồng bộ 2 chiều an toàn + đẩy lên cloud). Xung đột vẫn để bạn quyết định thủ công.</span>` → `<span>Tự động chạy định kỳ (chỉ thao tác an toàn). Có xung đột thì app sẽ mở cửa sổ để bạn duyệt.</span>`

- [ ] **Step 2: Sửa v2.js**

Xóa: biến `remotes`, `cloudLoaded`; nhánh `if (on && !cloudLoaded) loadCloud();` trong `showAdvanced`; hàm `doReplicate`, `loadCloud`, `renderTargets`, `wireTargetDialog`, `wireCloud`; trong `renderPairs` xóa dòng nút `<button class="btn btn-sm" data-a="repl">Đẩy cloud</button>` và dòng `row.querySelector('[data-a="repl"]').onclick = …`; trong `doApply` xóa block `if (r.replication && r.replication.length) { … }`; trong `init()` xóa các lời gọi `renderTargets(); wireTargetDialog(); wireCloud();` và sửa fallback `cfg = { pairs: [], targets: [] }` → `cfg = { pairs: [] }` (cả khai báo `let cfg` đầu file).

- [ ] **Step 3: Chạy thử app, kiểm tra tay**

Run: `npm run dev` (đợi cửa sổ mở, bấm "Nâng cao ▸")
Expected: tab nâng cao chỉ còn: Cặp đồng bộ, Tự động, Kết quả (ẩn), modal thêm cặp. Console webview không lỗi JS. Đóng app sau khi kiểm tra.

- [ ] **Step 4: Commit**

```bash
git add ui/index.html ui/v2.js && git commit -m "refactor(ui): gỡ toàn bộ UI cloud"
```

---

### Task 3: Chốt an toàn — chặn plan/apply khi thư mục mất kết nối

**Files:**
- Modify: `src-tauri/src/v2.rs` (thêm `dir_accessible`, `ensure_pair_accessible`, guard trong `plan`/`apply`, module test)

**Interfaces:**
- Consumes: `V2Pair { id, name, origin, working }`.
- Produces: `pub fn dir_accessible(p: &Path) -> bool`; `fn ensure_pair_accessible(p: &V2Pair) -> Result<(), String>`. Task 4-5 gọi `dir_accessible`.

- [ ] **Step 1: Viết test fail**

Thêm cuối `src-tauri/src/v2.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_accessible_phan_biet_ton_tai_va_khong() {
        let ok = std::env::temp_dir();
        assert!(dir_accessible(&ok));
        assert!(!dir_accessible(Path::new("/khong/ton/tai/chac/chan")));
    }

    #[test]
    fn ensure_pair_accessible_chan_khi_mot_phia_mat() {
        let tmp = std::env::temp_dir();
        let p = V2Pair {
            id: "t".into(),
            name: "t".into(),
            origin: tmp.to_string_lossy().to_string(),
            working: "/khong/ton/tai/usb-rut-ra".into(),
        };
        let err = ensure_pair_accessible(&p).unwrap_err();
        assert!(err.contains("không truy cập được"), "phải nêu rõ lý do: {}", err);

        let ok_pair = V2Pair { working: p.origin.clone(), ..p };
        assert!(ensure_pair_accessible(&ok_pair).is_ok());
    }
}
```

- [ ] **Step 2: Chạy test — phải FAIL**

Run: `cargo test --manifest-path src-tauri/Cargo.toml v2::tests`
Expected: FAIL compile — `dir_accessible`/`ensure_pair_accessible` chưa tồn tại.

- [ ] **Step 3: Cài đặt tối thiểu**

Thêm vào `v2.rs` (ngay trên `impl SyncManager`):

```rust
/// Thư mục còn "sống" không (USB còn cắm, ổ mạng còn nối)?
/// Dùng read_dir thay vì exists(): bắt được cả trường hợp mount point còn
/// nhưng không đọc được.
pub fn dir_accessible(p: &Path) -> bool {
    std::fs::read_dir(p).is_ok()
}

/// Chốt an toàn: scan_dir trả index RỖNG với thư mục không tồn tại — nếu không
/// chặn ở đây, rút USB sẽ bị hiểu là "xóa toàn bộ file" và lan sang phía kia.
fn ensure_pair_accessible(p: &V2Pair) -> Result<(), String> {
    if !dir_accessible(Path::new(&p.origin)) {
        return Err(format!("Thư mục gốc không truy cập được: {} — bỏ qua để tránh xóa nhầm", p.origin));
    }
    if !dir_accessible(Path::new(&p.working)) {
        return Err(format!("Bản làm việc không truy cập được: {} — bỏ qua để tránh xóa nhầm", p.working));
    }
    Ok(())
}
```

Rồi thêm guard ở đầu thân 2 hàm của `SyncManager` (sau dòng `let p = self.find_pair(id)?;`):

```rust
        ensure_pair_accessible(&p)?;
```

(cả trong `plan()` lẫn `apply()`; riêng `resolve()` cũng thêm — nó ghi file 2 phía.)

- [ ] **Step 4: Chạy test — phải PASS**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS toàn bộ (2 test mới + 5 test cũ).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/v2.rs && git commit -m "feat(v2): chốt an toàn — chặn plan/apply/resolve khi thư mục mất kết nối"
```

---

### Task 4: Watcher kết nối + event + command trạng thái

**Files:**
- Modify: `src-tauri/src/v2.rs` (state `conn`, `conn_transition`, `start_conn_watcher`, command `v2_conn_status`, test)
- Modify: `src-tauri/src/lib.rs` (gọi `start_conn_watcher`, đăng ký `v2_conn_status`)

**Interfaces:**
- Consumes: `dir_accessible` (Task 3).
- Produces: event `v2-conn-changed` payload `{pair_id: String, connected: bool}`; event `v2-reconnected` payload `{pair_id: String}`; command `v2_conn_status() -> Vec<ConnStatus{pair_id, connected}>`. Task 6 (UI) nghe các event/command này.

- [ ] **Step 1: Viết test fail cho máy trạng thái**

Thêm vào `mod tests` trong `v2.rs`:

```rust
    #[test]
    fn conn_transition_chi_ban_event_khi_doi_trang_thai() {
        // Lần quan sát đầu (prev=None): không bắn gì, kể cả đang mất kết nối.
        assert_eq!(conn_transition(None, true), None);
        assert_eq!(conn_transition(None, false), None);
        // Giữ nguyên trạng thái: không bắn.
        assert_eq!(conn_transition(Some(true), true), None);
        assert_eq!(conn_transition(Some(false), false), None);
        // Chuyển trạng thái: bắn đúng event.
        assert_eq!(conn_transition(Some(true), false), Some(ConnEvent::Disconnected));
        assert_eq!(conn_transition(Some(false), true), Some(ConnEvent::Reconnected));
    }
```

- [ ] **Step 2: Chạy test — phải FAIL**

Run: `cargo test --manifest-path src-tauri/Cargo.toml v2::tests::conn_transition_chi_ban_event_khi_doi_trang_thai`
Expected: FAIL compile — `conn_transition`/`ConnEvent` chưa tồn tại.

- [ ] **Step 3: Cài đặt watcher**

1. Import: thêm `Emitter` và `HashMap`:

```rust
use std::collections::HashMap;
use tauri::{AppHandle, Emitter, Manager, State};
```

2. Thêm field vào `SyncManager` + khởi tạo trong `new()`:

```rust
pub struct SyncManager {
    engine: Engine,
    cfg: V2Config,
    cfg_path: PathBuf,
    /// Trạng thái kết nối per-pair do watcher cập nhật (id -> connected).
    conn: HashMap<String, bool>,
}
```

(trong `new()`: `SyncManager { engine, cfg, cfg_path, conn: HashMap::new() }`)

3. Thêm máy trạng thái thuần + payload + watcher (đặt trên `start_scheduler`):

```rust
/// Sự kiện chuyển trạng thái kết nối của một cặp.
#[derive(Debug, PartialEq)]
pub enum ConnEvent {
    Disconnected,
    Reconnected,
}

/// prev=None là lần quan sát đầu tiên sau khi app mở — không bắn event
/// (tránh hiện thẻ duyệt oan mỗi lần khởi động).
pub fn conn_transition(prev: Option<bool>, connected: bool) -> Option<ConnEvent> {
    match (prev, connected) {
        (Some(true), false) => Some(ConnEvent::Disconnected),
        (Some(false), true) => Some(ConnEvent::Reconnected),
        _ => None,
    }
}

#[derive(Serialize, Clone)]
pub struct ConnPayload {
    pub pair_id: String,
    pub connected: bool,
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Watcher: poll 5s, chỉ read_dir thư mục gốc 2 phía. KHÔNG giữ lock
/// SyncManager trong lúc read_dir (ổ mạng chết có thể block rất lâu).
pub fn start_conn_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        let mut prev: HashMap<String, bool> = HashMap::new();
        loop {
            let pairs = {
                let st = app.state::<Mutex<SyncManager>>();
                let m = st.lock().unwrap();
                m.cfg.pairs.clone()
            };
            for p in &pairs {
                let connected = dir_accessible(Path::new(&p.origin))
                    && dir_accessible(Path::new(&p.working));
                let ev = conn_transition(prev.get(&p.id).copied(), connected);
                prev.insert(p.id.clone(), connected);
                {
                    let st = app.state::<Mutex<SyncManager>>();
                    st.lock().unwrap().conn.insert(p.id.clone(), connected);
                }
                match ev {
                    Some(ConnEvent::Disconnected) => {
                        let _ = app.emit("v2-conn-changed", ConnPayload {
                            pair_id: p.id.clone(),
                            connected: false,
                        });
                    }
                    Some(ConnEvent::Reconnected) => {
                        let _ = app.emit("v2-conn-changed", ConnPayload {
                            pair_id: p.id.clone(),
                            connected: true,
                        });
                        let _ = app.emit("v2-reconnected", ConnPayload {
                            pair_id: p.id.clone(),
                            connected: true,
                        });
                        show_main_window(&app);
                    }
                    None => {}
                }
            }
            prev.retain(|id, _| pairs.iter().any(|p| &p.id == id));
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
}

#[derive(Serialize)]
pub struct ConnStatus {
    pub pair_id: String,
    pub connected: bool,
}

#[tauri::command]
pub fn v2_conn_status(state: St) -> Vec<ConnStatus> {
    let m = state.lock().unwrap();
    m.cfg
        .pairs
        .iter()
        .map(|p| ConnStatus {
            pair_id: p.id.clone(),
            // Chưa quan sát lần nào -> coi như đang kết nối (watcher sẽ sửa sau ≤5s).
            connected: m.conn.get(&p.id).copied().unwrap_or(true),
        })
        .collect()
}
```

4. `lib.rs`: sau dòng `v2::start_scheduler(handle.clone());` thêm:

```rust
            v2::start_conn_watcher(handle.clone());
```

và thêm `v2::v2_conn_status,` vào `generate_handler!`.

- [ ] **Step 4: Chạy test — phải PASS**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS toàn bộ.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/v2.rs src-tauri/src/lib.rs && git commit -m "feat(v2): watcher kết nối 5s + event reconnect + command v2_conn_status"
```

---

### Task 5: Scheduler — bỏ qua cặp mất kết nối, báo conflict chờ duyệt

**Files:**
- Modify: `src-tauri/src/v2.rs` (`start_scheduler`)

**Interfaces:**
- Consumes: `SyncManager::conn`, `ApplyReport.conflicts`, `show_main_window` (Task 4).
- Produces: event `v2-conflicts` payload `{pair_id: String, count: usize}` — Task 6 nghe.

- [ ] **Step 1: Thay thân vòng lặp scheduler**

Thay block `for id in ids { … }` trong `start_scheduler` bằng:

```rust
        for id in ids {
            let report = {
                let mut m = st.lock().unwrap();
                // Cặp đang mất kết nối: bỏ qua chu kỳ này (guard trong apply
                // vẫn chặn thêm lần nữa nếu vừa rút ra sau khi kiểm tra).
                if !m.conn.get(&id).copied().unwrap_or(true) {
                    continue;
                }
                m.apply(&id) // chỉ thao tác an toàn; conflict không tự xử lý
            };
            if let Ok(r) = report {
                if !r.conflicts.is_empty() {
                    let _ = app.emit("v2-conflicts", ConflictPayload {
                        pair_id: id.clone(),
                        count: r.conflicts.len(),
                    });
                    show_main_window(&app);
                }
            }
        }
```

và thêm payload (cạnh `ConnPayload`):

```rust
#[derive(Serialize, Clone)]
pub struct ConflictPayload {
    pub pair_id: String,
    pub count: usize,
}
```

- [ ] **Step 2: Build + test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS, không warning unused.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/v2.rs && git commit -m "feat(v2): scheduler bỏ qua cặp mất kết nối, bắn event khi có conflict"
```

---

### Task 6: UI — badge kết nối + thẻ duyệt khi reconnect/conflict

**Files:**
- Modify: `ui/v2.js`

**Interfaces:**
- Consumes: event `v2-conn-changed`/`v2-reconnected`/`v2-conflicts` (payload `{pair_id, connected|count}`), command `v2_conn_status`, `v2_plan`, `v2_apply` (Task 1-5).
- Produces: —

- [ ] **Step 1: Thêm state + badge kết nối vào renderPairs**

Đầu IIFE (cạnh `let cfg = { pairs: [] };`):

```js
  const { listen } = window.__TAURI__.event;
  let connMap = {}; // pair_id -> connected (từ v2_conn_status + event)
```

Trong `renderPairs()`, thay dòng `<div class="pair-path"><b>${esc(p.name)}</b></div>` bằng:

```js
          <div class="pair-path"><b>${esc(p.name)}</b>
            <span id="conn-${esc(p.id)}" style="font-size:11.5px;font-weight:600;margin-left:6px;color:${
              connMap[p.id] === false ? "var(--red)" : "var(--green)"
            }">${connMap[p.id] === false ? "● Mất kết nối" : "● Đang kết nối"}</span></div>
```

Thêm hàm nạp/cập nhật:

```js
  async function loadConn() {
    try {
      const list = await invoke("v2_conn_status");
      connMap = {};
      for (const s of list) connMap[s.pair_id] = s.connected;
      renderPairs();
    } catch (e) { /* chưa có pair nào cũng không sao */ }
  }
```

- [ ] **Step 2: Thẻ duyệt — thêm nút "Duyệt & Đồng bộ" vào kết quả plan**

Sửa chữ ký `renderOpsAndConflicts(box, pairId, name, ops, conflicts)` thành `renderOpsAndConflicts(box, pairId, name, ops, conflicts, withApprove)` và trước dòng `box.innerHTML = html;` thêm:

```js
    if (withApprove && ops.length) {
      html += `<div style="margin-top:12px;display:flex;gap:8px">
        <button class="btn btn-primary" id="v2Approve">✓ Duyệt &amp; Đồng bộ (${ops.length} thay đổi)</button>
        <button class="btn" id="v2Dismiss">Bỏ qua</button></div>`;
    }
```

và sau khi gán `box.innerHTML = html;` thêm:

```js
    if (withApprove && ops.length) {
      $("v2Approve").onclick = () => doApply(pairId, name);
      $("v2Dismiss").onclick = () => { $("v2ResultCard").style.display = "none"; };
    }
```

Sửa `doPlan(pairId, name)` thành `doPlan(pairId, name, withApprove)` và truyền tiếp: `renderOpsAndConflicts(box, pairId, name, plan.ops, plan.conflicts, withApprove);`. (Nút "Kiểm tra" trong `renderPairs` gọi `doPlan(p.id, p.name, true)` — kiểm tra tay cũng nên duyệt được ngay.)

- [ ] **Step 3: Nghe event**

Thêm hàm + gọi trong `init()`:

```js
  function pairName(id) {
    const p = cfg.pairs.find((x) => x.id === id);
    return p ? p.name : id;
  }

  function wireEvents() {
    listen("v2-conn-changed", (e) => {
      connMap[e.payload.pair_id] = e.payload.connected;
      renderPairs();
    });
    // Reconnect: KHÔNG tự sync — tính plan và chờ người dùng duyệt.
    listen("v2-reconnected", async (e) => {
      showAdvanced(true);
      const id = e.payload.pair_id;
      const box = showResult(`Kết nối lại: ${pairName(id)} — xem & duyệt thay đổi`);
      box.innerHTML = "<p class='empty'>Đang quét…</p>";
      await doPlan(id, pairName(id), true);
    });
    // Định kỳ gặp conflict: mở thẻ duyệt để xử lý từng file.
    listen("v2-conflicts", (e) => {
      showAdvanced(true);
      doPlan(e.payload.pair_id, pairName(e.payload.pair_id), true);
    });
  }
```

Trong `init()`: thêm `wireEvents();` và `loadConn();` (sau `renderPairs();`).

- [ ] **Step 4: Kiểm tra tay luồng reconnect**

Run: `npm run dev`, tạo 1 cặp trỏ vào 2 thư mục tạm, ví dụ `/tmp/goc` và `/tmp/lamviec`, bỏ vài file vào `/tmp/goc`, bấm Đồng bộ lần đầu. Sau đó:
1. `mv /tmp/lamviec /tmp/lamviec_x` → trong ≤5s badge cặp chuyển "● Mất kết nối" (đỏ); bấm "Kiểm tra" phải báo lỗi "không truy cập được".
2. Sửa 1 file trong `/tmp/goc`, rồi `mv /tmp/lamviec_x /tmp/lamviec` → cửa sổ tự bật lên, thẻ "Kết nối lại: … — xem & duyệt thay đổi" hiện plan + nút "✓ Duyệt & Đồng bộ".
3. Bấm Duyệt → báo "Xong · Copy: 1…"; bấm lại "Kiểm tra" → "Không có thay đổi — đã đồng bộ."

Expected: đúng cả 3 bước, không lỗi console.

- [ ] **Step 5: Commit**

```bash
git add ui/v2.js && git commit -m "feat(ui): badge kết nối + thẻ duyệt khi reconnect và khi định kỳ gặp conflict"
```

---

### Task 7: UI — lịch sử phiên bản (time travel)

**Files:**
- Modify: `ui/index.html` (thêm section Lịch sử)
- Modify: `ui/v2.js` (render + gọi `v2_history`/`v2_restore_version`)

**Interfaces:**
- Consumes: command `v2_history(id, rel) -> [{id, created_at(µs), op, size}]`, `v2_restore_version(versionId, dst)`, `pick_folder` (đều có sẵn).
- Produces: —

- [ ] **Step 1: Thêm section vào index.html**

Chèn sau section `<!-- Kết quả -->` trong `#tab-v2`:

```html
        <!-- Lịch sử phiên bản -->
        <section class="card">
          <div class="card-head"><h2>Lịch sử phiên bản (khôi phục về quá khứ)</h2></div>
          <div class="pick" style="gap:8px">
            <select id="v2hPair" class="num" style="min-width:160px"></select>
            <input id="v2hRel" type="text" placeholder="Đường dẫn tương đối, VD: baocao/thang7.xlsx" />
            <button id="v2hShow" class="btn btn-sm">Xem lịch sử</button>
          </div>
          <div id="v2History"></div>
          <p class="empty" style="text-align:left;font-size:12.5px">
            Mọi file bị ghi đè/xóa khi đồng bộ đều được giữ lại tự động — chọn phiên bản để khôi phục ra thư mục bạn muốn.
          </p>
        </section>
```

- [ ] **Step 2: Thêm logic vào v2.js**

```js
  // ---------- Lịch sử phiên bản (time travel) ----------
  function renderHistoryPairs() {
    const sel = $("v2hPair");
    sel.innerHTML = cfg.pairs.map((p) => `<option value="${esc(p.id)}">${esc(p.name)}</option>`).join("");
  }

  function fmtTime(micros) {
    return new Date(micros / 1000).toLocaleString("vi-VN");
  }
  function fmtSize(b) {
    if (b >= 1048576) return (b / 1048576).toFixed(1) + " MB";
    if (b >= 1024) return (b / 1024).toFixed(1) + " KB";
    return b + " B";
  }

  function wireHistory() {
    $("v2hShow").onclick = async () => {
      const pairId = $("v2hPair").value;
      const rel = $("v2hRel").value.trim();
      const box = $("v2History");
      if (!pairId || !rel) return alert("Chọn cặp và nhập đường dẫn tương đối của file.");
      box.innerHTML = "<p class='empty'>Đang tải…</p>";
      try {
        const list = await invoke("v2_history", { id: pairId, rel });
        if (!list.length) {
          box.innerHTML = "<p class='empty' style='text-align:left'>Chưa có phiên bản nào được lưu cho file này.</p>";
          return;
        }
        box.innerHTML = "";
        for (const v of list) {
          const row = document.createElement("div");
          row.className = "v2-line";
          row.innerHTML = `<span class="v2-badge ${v.op === "delete" ? "v2-del" : "v2-copy"}">${
            v.op === "delete" ? "Trước khi xóa" : "Trước khi ghi đè"
          }</span>
            <span>${fmtTime(v.created_at)} · ${fmtSize(v.size)}</span>
            <button class="btn btn-sm" style="margin-left:auto">Khôi phục…</button>`;
          row.querySelector("button").onclick = async () => {
            const folder = await invoke("pick_folder");
            if (!folder) return;
            const base = rel.split("/").pop();
            const dst = folder + "/" + "khoi-phuc-" + base;
            try {
              await invoke("v2_restore_version", { versionId: v.id, dst });
              alert("Đã khôi phục ra: " + dst);
            } catch (e) {
              alert("Không khôi phục được: " + e);
            }
          };
          box.appendChild(row);
        }
      } catch (e) {
        box.innerHTML = `<p class="empty" style="color:var(--red)">Lỗi: ${esc(e)}</p>`;
      }
    };
  }
```

Trong `init()`: thêm `renderHistoryPairs(); wireHistory();`. Trong `renderPairs()` — cuối hàm gọi `renderHistoryPairs();` để select luôn khớp danh sách cặp.

- [ ] **Step 3: Kiểm tra tay**

Run: `npm run dev` — với cặp test ở Task 6: sửa 1 file working rồi Đồng bộ (để sinh version "overwrite"). Vào Lịch sử: chọn cặp, nhập đường dẫn tương đối file đó, "Xem lịch sử" → thấy ≥1 dòng "Trước khi ghi đè"; bấm "Khôi phục…" chọn `/tmp` → file `khoi-phuc-<tên>` xuất hiện đúng nội dung cũ.
Expected: đúng như trên, không lỗi console.

- [ ] **Step 4: Commit**

```bash
git add ui/index.html ui/v2.js && git commit -m "feat(ui): xem lịch sử phiên bản + khôi phục file (time travel)"
```

---

### Task 8: Bump 0.3.0 + hoàn tất

**Files:**
- Modify: `src-tauri/Cargo.toml:3`, `src-tauri/tauri.conf.json:4`, `package.json:4` (version → `0.3.0`)

**Interfaces:** —

- [ ] **Step 1: Đổi version 3 file**

`"version": "0.2.1"` → `"version": "0.3.0"` trong `tauri.conf.json` + `package.json`; `version = "0.2.1"` → `version = "0.3.0"` trong `Cargo.toml`.

- [ ] **Step 2: Build release check + full test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && cargo check --manifest-path src-tauri/Cargo.toml --release`
Expected: PASS / no error. (Lưu ý: `Cargo.lock` sẽ tự cập nhật version — add cả nó.)

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "chore: bump 0.3.0 — reconnect + duyệt sync, bỏ cloud"
```

---

## Self-Review (đã chạy)

1. **Spec coverage:** §1 gỡ cloud → Task 1-2; §2 watcher → Task 4; §3 chốt an toàn → Task 3; §4 scheduler → Task 5; §5 time travel UI → Task 7; §6 badge kết nối → Task 6 (badge inline trong renderPairs + loadConn thay vì command riêng lúc mở tab — cùng hành vi spec); version bump → Task 8. Không còn mục nào thiếu.
2. **Placeholder:** không có TBD/TODO; mọi step có code/command cụ thể.
3. **Type consistency:** `ConnPayload{pair_id, connected}` khớp JS `e.payload.pair_id`; `ConflictPayload{pair_id, count}`; `v2_conn_status -> Vec<ConnStatus{pair_id, connected}>` khớp `loadConn()`; `VersionDto{id, created_at, op, size}` khớp `wireHistory()`; `doPlan(pairId, name, withApprove)` khớp mọi call-site được sửa.
