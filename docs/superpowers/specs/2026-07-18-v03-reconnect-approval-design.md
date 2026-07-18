# Thiết kế v0.3.0 — Reconnect + Duyệt sync, bỏ cloud

Ngày: 2026-07-18 · Trạng thái: đã duyệt

## Mục tiêu

1. **Giữ nguyên** workflow v1 `original → backup` (backup.rs / engine.rs / tab đơn giản).
2. Workflow v2 `original ↔ working`:
   - Tự phát hiện thư mục working (hoặc origin) **mất kết nối / kết nối lại**.
   - Khi **kết nối lại**: hiện plan đầy đủ, **người dùng duyệt mới sync** (không bao giờ tự chạy).
   - Sync **2 chiều** (đã có sẵn trong engine).
   - Luôn giữ bản an toàn của file bị ghi đè/xóa để **time travel** (version store đã có; bổ sung UI xem lịch sử).
3. **Bỏ hoàn toàn cloud** (rclone).

## Phạm vi KHÔNG đổi

- Engine v1 (`backup.rs`, `engine.rs`, `config.rs`) và tab "chế độ đơn giản".
- Lõi engine v2 (`sync/mod.rs`, `db.rs`, `diff.rs`, `scan.rs`, `store.rs`, `types.rs`): diff 3 phía,
  conflict không tự xử lý, version store + journal undo.

## Thay đổi

### 1. Gỡ cloud

- Xóa file `src-tauri/src/sync/cloud.rs`; bỏ `pub mod cloud` khỏi `sync/mod.rs`.
- `V2Config`: bỏ field `targets`. Config cũ trên máy người dùng có `targets` vẫn parse được
  (serde mặc định bỏ qua field lạ).
- `SyncManager`: bỏ field `rclone`, hàm `replicate`.
- Bỏ các command: `v2_status`, `v2_add_target`, `v2_remove_target`, `v2_replicate`,
  `v2_connect_remote` (cả trong `generate_handler!`).
- `ApplyReport`: bỏ field `replication`, struct `ReplStatus`, `V2Status`.
- UI: xóa section cloud (status rclone, connect Drive/OneDrive, danh sách target, nút "Đẩy cloud")
  khỏi `ui/index.html` và `ui/v2.js`.
- `Cargo.toml`: gỡ dependency chỉ dùng cho rclone nếu có (kiểm tra khi triển khai).

### 2. Watcher kết nối (mới, trong `v2.rs`)

- Thread nền poll mỗi **5 giây**. Với mỗi cặp: `connected = dir_accessible(origin) && dir_accessible(working)`;
  `dir_accessible` = `std::fs::read_dir(p).is_ok()` (bắt được cả USB rút ra lẫn ổ mạng rớt,
  không chỉ kiểm tra tồn tại).
- Trạng thái per-pair lưu trong `SyncManager` (map `pair_id → bool`), expose qua command mới
  `v2_conn_status() → Vec<{pair_id, connected}>`.
- Chuyển **connected → disconnected**: emit event `v2-conn-changed {pair_id, connected: false}`.
- Chuyển **disconnected → connected** (chỉ khi trước đó ĐÃ từng thấy disconnected, không bắn lúc
  app mới mở): emit event `v2-reconnected {pair_id}` + gọi `open_main` để đưa cửa sổ lên.
- UI nghe `v2-reconnected` → gọi `v2_plan` → render **thẻ duyệt** (danh sách ops + conflicts,
  nút "Duyệt & Đồng bộ" / "Bỏ qua"). Chỉ khi bấm Duyệt mới gọi `v2_apply`.

### 3. Chốt an toàn (mới)

- `SyncManager::plan` và `apply` kiểm tra `dir_accessible` cả 2 phía trước khi chạy; nếu một phía
  không truy cập được → `Err("thư mục ... không truy cập được — bỏ qua để tránh xóa nhầm")`.
- Lý do: `scan_dir` trả index rỗng với thư mục không tồn tại → nếu không chặn, rút ổ USB sẽ bị
  hiểu là "xóa toàn bộ file" và lan sang phía kia.

### 4. Scheduler định kỳ (sửa)

- Giữ vòng lặp định kỳ (`auto` + `interval_minutes` như cũ).
- Mỗi chu kỳ, với từng cặp: nếu mất kết nối → bỏ qua; ngược lại `apply` như cũ
  (engine chỉ tự chạy thao tác an toàn, conflict không đụng).
- Nếu kết quả có **conflict** → emit event `v2-conflicts {pair_id, count}` + `open_main`;
  UI hiện thẻ duyệt để người dùng xử lý từng file (Giữ Gốc / Giữ Bản làm việc — đã có sẵn).

### 5. UI time travel (bổ sung nhỏ)

- Thêm vào tab nâng cao: ô nhập đường dẫn tương đối + nút "Xem lịch sử" → gọi `v2_history`,
  hiện danh sách phiên bản (thời gian, thao tác, kích thước) + nút "Khôi phục…" → chọn nơi lưu
  (`pick_folder` + tên file) → `v2_restore_version`.
- Giữ nút "Hoàn tác lần đồng bộ gần nhất".

### 6. Hiển thị trạng thái kết nối

- Mỗi cặp trong danh sách hiện badge "Đang kết nối" / "Mất kết nối" (từ `v2_conn_status`
  lúc mở tab + cập nhật theo event `v2-conn-changed`).

## Luồng dữ liệu chính

```
Watcher (5s) ──> trạng thái per-pair ──> event v2-conn-changed / v2-reconnected
                                              │
UI: thẻ duyệt <── v2_plan (chặn nếu mất kết nối)
      │ Duyệt
      └──> v2_apply ──> engine.apply (cất version store trước khi ghi/xóa) ──> journal (undo)
Scheduler (interval) ──> apply an toàn ──> nếu conflict: event v2-conflicts ──> thẻ duyệt
```

## Xử lý lỗi

- Mất kết nối giữa chừng khi apply: thao tác copy là atomic (temp + rename); file lỗi IO → dừng
  và trả lỗi, các thao tác đã xong vẫn undo được qua `run_id`.
- Event emit thất bại (cửa sổ đóng): không chặn watcher; UI đọc lại `v2_conn_status` khi mở.

## Kiểm thử

- Rust: test cho chốt an toàn (working root không tồn tại → plan/apply trả Err, origin không bị
  xóa file); test watcher logic chuyển trạng thái (tách hàm thuần `next_state(prev, connected)`).
- Chạy tay: rút/cắm thư mục giả lập (đổi tên thư mục working) → cửa sổ bật lên với thẻ duyệt.

## Phiên bản

- Bump `0.3.0` (tauri.conf.json, Cargo.toml, package.json nếu có version).
