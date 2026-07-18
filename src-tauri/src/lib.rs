mod backup;
mod config;
mod engine;
mod logger;
mod sync;
mod v2;

use config::{Config, Mode, Pair};
use engine::Engine;
use logger::Logger;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_dialog::DialogExt;

/// Trạng thái hiển thị cho UI.
#[derive(Clone, Serialize)]
pub struct Status {
    pub running: bool,
    pub mode: String,
    pub activity: String, // "idle" | "syncing"
    pub interval_minutes: u64,
    pub pairs: usize,
    pub last_run: Option<String>,
    pub last_summary: String,
}

impl Status {
    fn from_cfg(cfg: &Config) -> Self {
        Status {
            running: cfg.running,
            mode: match cfg.mode {
                Mode::Realtime => "realtime".into(),
                Mode::Periodic => "periodic".into(),
            },
            activity: "idle".into(),
            interval_minutes: cfg.interval_minutes,
            pairs: cfg.pairs.iter().filter(|p| p.enabled).count(),
            last_run: None,
            last_summary: String::new(),
        }
    }
}

pub struct AppState {
    pub config: Mutex<Config>,
    pub status: Arc<Mutex<Status>>,
    pub logger: Arc<Logger>,
    pub engine: Mutex<Option<Engine>>,
}

fn gen_id() -> String {
    format!("p{}", chrono::Local::now().timestamp_micros())
}

fn reload_engine(state: &AppState, cfg: &Config) {
    if let Some(e) = state.engine.lock().unwrap().as_ref() {
        e.reload(cfg.clone());
    }
}

// ---------------- Commands ----------------

#[tauri::command]
fn get_config(state: State<AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn get_status(state: State<AppState>) -> Status {
    state.status.lock().unwrap().clone()
}

#[tauri::command]
fn get_logs(state: State<AppState>) -> Vec<String> {
    state.logger.read_recent()
}

/// Mở hộp thoại chọn thư mục (chạy phía Rust để tương thích mọi HĐH).
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |p| {
        let path = p.and_then(|fp| fp.into_path().ok())
            .map(|pb| pb.to_string_lossy().to_string());
        let _ = tx.send(path);
    });
    tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .ok()
        .flatten()
}

#[tauri::command]
fn add_pair(app: AppHandle, state: State<AppState>, source: String, dest: String, mirror: bool) -> Config {
    let cfg = {
        let mut cfg = state.config.lock().unwrap();
        cfg.pairs.push(Pair {
            id: gen_id(),
            source: source.clone(),
            dest: dest.clone(),
            mirror,
            enabled: true,
        });
        config::save(&app, &cfg);
        cfg.clone()
    };
    state.logger.log("INFO", &format!("Thêm cặp sao lưu: {} -> {}", source, dest));
    reload_engine(&state, &cfg);
    cfg
}

#[tauri::command]
fn remove_pair(app: AppHandle, state: State<AppState>, id: String) -> Config {
    let cfg = {
        let mut cfg = state.config.lock().unwrap();
        cfg.pairs.retain(|p| p.id != id);
        config::save(&app, &cfg);
        cfg.clone()
    };
    state.logger.log("INFO", &format!("Xóa cặp sao lưu: {}", id));
    reload_engine(&state, &cfg);
    cfg
}

#[tauri::command]
fn toggle_pair(app: AppHandle, state: State<AppState>, id: String, enabled: bool) -> Config {
    let cfg = {
        let mut cfg = state.config.lock().unwrap();
        if let Some(p) = cfg.pairs.iter_mut().find(|p| p.id == id) {
            p.enabled = enabled;
        }
        config::save(&app, &cfg);
        cfg.clone()
    };
    reload_engine(&state, &cfg);
    cfg
}

#[tauri::command]
fn set_pair_mirror(app: AppHandle, state: State<AppState>, id: String, mirror: bool) -> Config {
    let cfg = {
        let mut cfg = state.config.lock().unwrap();
        if let Some(p) = cfg.pairs.iter_mut().find(|p| p.id == id) {
            p.mirror = mirror;
        }
        config::save(&app, &cfg);
        cfg.clone()
    };
    reload_engine(&state, &cfg);
    cfg
}

#[tauri::command]
fn set_mode(app: AppHandle, state: State<AppState>, mode: String, interval_minutes: u64) -> Config {
    let cfg = {
        let mut cfg = state.config.lock().unwrap();
        cfg.mode = if mode == "realtime" { Mode::Realtime } else { Mode::Periodic };
        cfg.interval_minutes = interval_minutes.max(1);
        config::save(&app, &cfg);
        cfg.clone()
    };
    reload_engine(&state, &cfg);
    cfg
}

#[tauri::command]
fn set_running(app: AppHandle, state: State<AppState>, running: bool) -> Config {
    let cfg = {
        let mut cfg = state.config.lock().unwrap();
        cfg.running = running;
        config::save(&app, &cfg);
        cfg.clone()
    };
    if let Some(e) = state.engine.lock().unwrap().as_ref() {
        e.set_running(running);
    }
    cfg
}

#[tauri::command]
fn set_autostart(app: AppHandle, state: State<AppState>, enabled: bool) -> Config {
    let cfg = {
        let mut cfg = state.config.lock().unwrap();
        cfg.autostart = enabled;
        config::save(&app, &cfg);
        cfg.clone()
    };
    let al = app.autolaunch();
    let _ = if enabled { al.enable() } else { al.disable() };
    cfg
}

#[tauri::command]
fn backup_now(state: State<AppState>) {
    if let Some(e) = state.engine.lock().unwrap().as_ref() {
        e.run_now();
    }
}

#[tauri::command]
fn show_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

// ---------------- Setup ----------------

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItemBuilder::with_id("show", "Mở cửa sổ").build(app)?;
    let backup = MenuItemBuilder::with_id("backup", "Sao lưu ngay").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Thoát").build(app)?;
    let menu = MenuBuilder::new(app).items(&[&show, &backup, &quit]).build()?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Backup Helper — Sao lưu dữ liệu")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => open_main(app),
            "backup" => {
                let state = app.state::<AppState>();
                let guard = state.engine.lock().unwrap();
                if let Some(e) = guard.as_ref() {
                    e.run_now();
                }
            }
            "quit" => {
                let state = app.state::<AppState>();
                {
                    let guard = state.engine.lock().unwrap();
                    if let Some(e) = guard.as_ref() {
                        e.shutdown();
                    }
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                open_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn open_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .setup(|app| {
            let handle = app.handle().clone();
            let logger = Arc::new(Logger::new(&handle));
            logger.cleanup();

            let cfg = config::load(&handle);

            // Đồng bộ trạng thái tự khởi động với cấu hình.
            let al = app.autolaunch();
            let _ = if cfg.autostart { al.enable() } else { al.disable() };

            let status = Arc::new(Mutex::new(Status::from_cfg(&cfg)));
            let eng = Engine::start(handle.clone(), cfg.clone(), logger.clone(), status.clone());

            app.manage(AppState {
                config: Mutex::new(cfg),
                status,
                logger,
                engine: Mutex::new(Some(eng)),
            });

            // State v2: engine đồng bộ 2 chiều.
            app.manage(Mutex::new(v2::init(&handle)));
            v2::start_scheduler(handle.clone());

            build_tray(app)?;

            // Đóng cửa sổ = thu nhỏ xuống khay, không thoát app.
            if let Some(win) = app.get_webview_window("main") {
                let w = win.clone();
                win.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
                // Khởi động cùng HĐH thì chạy ẩn dưới khay.
                if std::env::args().any(|a| a == "--minimized") {
                    let _ = win.hide();
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            get_status,
            get_logs,
            pick_folder,
            add_pair,
            remove_pair,
            toggle_pair,
            set_pair_mirror,
            set_mode,
            set_running,
            set_autostart,
            backup_now,
            show_window,
            v2::v2_get_config,
            v2::v2_add_pair,
            v2::v2_remove_pair,
            v2::v2_plan,
            v2::v2_apply,
            v2::v2_resolve,
            v2::v2_undo,
            v2::v2_undo_last,
            v2::v2_history,
            v2::v2_restore_version,
            v2::v2_set_auto
        ])
        .run(tauri::generate_context!())
        .expect("lỗi khi chạy ứng dụng Backup Helper");
}
