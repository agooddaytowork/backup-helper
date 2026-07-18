//! Lớp wiring v2: nối engine đồng bộ 2 chiều vào Tauri.
//! Quản lý cấu hình cặp origin↔working, điều phối plan/apply/resolve/undo/history.

use crate::sync::types::{Conflict, Direction, OpKind, Plan, PlannedOp, Side};
use crate::sync::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Serialize, Deserialize, Clone)]
pub struct V2Pair {
    pub id: String,
    pub name: String,
    pub origin: String,
    pub working: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct V2Config {
    #[serde(default)]
    pub pairs: Vec<V2Pair>,
    #[serde(default)]
    pub last_run: Option<String>,
    /// Tự động đồng bộ + đẩy cloud định kỳ.
    #[serde(default)]
    pub auto: bool,
    #[serde(default = "d_interval")]
    pub interval_minutes: u64,
}

fn d_interval() -> u64 {
    30
}

impl Default for V2Config {
    fn default() -> Self {
        V2Config {
            pairs: vec![],
            last_run: None,
            auto: false,
            interval_minutes: 30,
        }
    }
}

pub struct SyncManager {
    engine: Engine,
    cfg: V2Config,
    cfg_path: PathBuf,
    /// Trạng thái kết nối per-pair do watcher cập nhật (id -> connected).
    conn: HashMap<String, bool>,
}

// ---------- DTO trả về UI ----------

#[derive(Serialize)]
pub struct ApplyReport {
    pub run_id: String,
    pub copied: usize,
    pub deleted: usize,
    pub conflicts: Vec<Conflict>,
}

type R<T> = Result<T, String>;

fn now_id() -> String {
    format!("p{}", chrono::Local::now().timestamp_micros())
}

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
        return Err(format!(
            "Thư mục gốc không truy cập được: {} — bỏ qua để tránh xóa nhầm",
            p.origin
        ));
    }
    if !dir_accessible(Path::new(&p.working)) {
        return Err(format!(
            "Bản làm việc không truy cập được: {} — bỏ qua để tránh xóa nhầm",
            p.working
        ));
    }
    Ok(())
}

impl SyncManager {
    pub fn new(engine: Engine, cfg_path: PathBuf) -> SyncManager {
        let cfg = std::fs::read_to_string(&cfg_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        SyncManager { engine, cfg, cfg_path, conn: HashMap::new() }
    }

    fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(&self.cfg) {
            let _ = std::fs::write(&self.cfg_path, s);
        }
    }

    fn find_pair(&self, id: &str) -> R<V2Pair> {
        self.cfg
            .pairs
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| "không tìm thấy cặp thư mục".to_string())
    }

    fn plan(&self, id: &str) -> R<Plan> {
        let p = self.find_pair(id)?;
        ensure_pair_accessible(&p)?;
        self.engine
            .plan(&p.id, Path::new(&p.origin), Path::new(&p.working))
            .map_err(|e| e.to_string())
    }

    fn apply(&mut self, id: &str) -> R<ApplyReport> {
        let p = self.find_pair(id)?;
        ensure_pair_accessible(&p)?;
        let plan = self
            .engine
            .plan(&p.id, Path::new(&p.origin), Path::new(&p.working))
            .map_err(|e| e.to_string())?;
        let copied = plan
            .ops
            .iter()
            .filter(|o| matches!(o.kind, OpKind::Copy(_)))
            .count();
        let deleted = plan
            .ops
            .iter()
            .filter(|o| matches!(o.kind, OpKind::Delete(_)))
            .count();
        let run_id = self
            .engine
            .apply(&p.id, Path::new(&p.origin), Path::new(&p.working), &plan)
            .map_err(|e| e.to_string())?;
        self.cfg.last_run = Some(run_id.clone());
        self.save();

        Ok(ApplyReport {
            run_id,
            copied,
            deleted,
            conflicts: plan.conflicts,
        })
    }

    /// Giải quyết 1 xung đột theo lựa chọn của người dùng.
    fn resolve(&mut self, id: &str, rel: &str, keep: Side) -> R<String> {
        let p = self.find_pair(id)?;
        ensure_pair_accessible(&p)?;
        let origin = Path::new(&p.origin);
        let working = Path::new(&p.working);
        let origin_has = origin.join(rel).exists();
        let working_has = working.join(rel).exists();

        // Suy ra thao tác từ (giữ bên nào) + (bên đó còn file hay không).
        let kind = match keep {
            Side::Working => {
                if working_has {
                    OpKind::Copy(Direction::WorkingToOrigin)
                } else {
                    OpKind::Delete(Side::Origin)
                }
            }
            Side::Origin => {
                if origin_has {
                    OpKind::Copy(Direction::OriginToWorking)
                } else {
                    OpKind::Delete(Side::Working)
                }
            }
        };
        let plan = Plan {
            ops: vec![PlannedOp {
                rel_path: rel.to_string(),
                kind,
            }],
            conflicts: vec![],
        };
        self.engine
            .apply(&p.id, origin, working, &plan)
            .map_err(|e| e.to_string())
    }
}

// ================= Tauri commands =================

type St<'a> = State<'a, Mutex<SyncManager>>;

#[tauri::command]
pub fn v2_get_config(state: St) -> V2Config {
    state.lock().unwrap().cfg.clone()
}

#[tauri::command]
pub fn v2_add_pair(state: St, name: String, origin: String, working: String) -> R<V2Config> {
    if origin == working {
        return Err("Nguồn và đích không được trùng".into());
    }
    let mut m = state.lock().unwrap();
    m.cfg.pairs.push(V2Pair {
        id: now_id(),
        name: if name.is_empty() { "Cặp thư mục".into() } else { name },
        origin,
        working,
    });
    m.save();
    Ok(m.cfg.clone())
}

#[tauri::command]
pub fn v2_remove_pair(state: St, id: String) -> V2Config {
    let mut m = state.lock().unwrap();
    m.cfg.pairs.retain(|p| p.id != id);
    m.save();
    m.cfg.clone()
}

#[tauri::command]
pub fn v2_plan(state: St, id: String) -> R<Plan> {
    state.lock().unwrap().plan(&id)
}

#[tauri::command]
pub fn v2_apply(state: St, id: String) -> R<ApplyReport> {
    state.lock().unwrap().apply(&id)
}

#[tauri::command]
pub fn v2_resolve(state: St, id: String, rel: String, keep: String) -> R<String> {
    let side = match keep.as_str() {
        "working" => Side::Working,
        "origin" => Side::Origin,
        _ => return Err("lựa chọn không hợp lệ".into()),
    };
    state.lock().unwrap().resolve(&id, &rel, side)
}

#[tauri::command]
pub fn v2_undo(state: St, run_id: String) -> R<()> {
    state.lock().unwrap().engine.undo(&run_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn v2_undo_last(state: St) -> R<String> {
    let m = state.lock().unwrap();
    let run = m.cfg.last_run.clone().ok_or("chưa có lần đồng bộ nào để hoàn tác")?;
    m.engine.undo(&run).map_err(|e| e.to_string())?;
    Ok(run)
}

#[derive(Serialize)]
pub struct VersionDto {
    pub id: i64,
    pub created_at: i64,
    pub op: String,
    pub size: i64,
}

#[tauri::command]
pub fn v2_history(state: St, id: String, rel: String) -> R<Vec<VersionDto>> {
    let m = state.lock().unwrap();
    let p = m.find_pair(&id)?;
    let vs = m.engine.history(&p.id, &rel).map_err(|e| e.to_string())?;
    Ok(vs
        .into_iter()
        .map(|v| VersionDto {
            id: v.id,
            created_at: v.created_at,
            op: v.op,
            size: v.size,
        })
        .collect())
}

#[tauri::command]
pub fn v2_restore_version(state: St, version_id: i64, dst: String) -> R<()> {
    state
        .lock()
        .unwrap()
        .engine
        .restore_version(version_id, Path::new(&dst))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn v2_set_auto(state: St, auto: bool, interval_minutes: u64) -> V2Config {
    let mut m = state.lock().unwrap();
    m.cfg.auto = auto;
    m.cfg.interval_minutes = interval_minutes.max(1);
    m.save();
    m.cfg.clone()
}

// ================= Watcher kết nối =================

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
                        let _ = app.emit(
                            "v2-conn-changed",
                            ConnPayload { pair_id: p.id.clone(), connected: false },
                        );
                    }
                    Some(ConnEvent::Reconnected) => {
                        let _ = app.emit(
                            "v2-conn-changed",
                            ConnPayload { pair_id: p.id.clone(), connected: true },
                        );
                        // Reconnect KHÔNG tự sync: chỉ báo UI hiện thẻ duyệt.
                        let _ = app.emit(
                            "v2-reconnected",
                            ConnPayload { pair_id: p.id.clone(), connected: true },
                        );
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

/// Luồng nền: định kỳ chạy đồng bộ cho mọi cặp (nếu bật tự động).
/// Xung đột KHÔNG tự xử lý — vẫn để người dùng quyết định.
pub fn start_scheduler(app: AppHandle) {
    std::thread::spawn(move || loop {
        let interval = {
            let st = app.state::<Mutex<SyncManager>>();
            let m = st.lock().unwrap();
            m.cfg.interval_minutes.max(1)
        };
        std::thread::sleep(std::time::Duration::from_secs(interval * 60));

        let st = app.state::<Mutex<SyncManager>>();
        let (auto, ids) = {
            let m = st.lock().unwrap();
            (
                m.cfg.auto,
                m.cfg.pairs.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
            )
        };
        if !auto {
            continue;
        }
        for id in ids {
            let mut m = st.lock().unwrap();
            let _ = m.apply(&id); // apply chỉ chạy thao tác an toàn; conflict để người dùng quyết
        }
    });
}

/// Khởi tạo state v2 trong setup của Tauri.
pub fn init(app: &AppHandle) -> SyncManager {
    let base = app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."));
    let v2dir = base.join("v2");
    let _ = std::fs::create_dir_all(&v2dir);
    let engine = Engine::open(&v2dir.join("meta.db"), &v2dir.join("store"))
        .expect("không mở được engine v2");
    SyncManager::new(engine, v2dir.join("v2-config.json"))
}

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
}
