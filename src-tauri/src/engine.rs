use crate::backup;
use crate::config::{Config, Mode};
use crate::logger::Logger;
use crate::Status;
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, Debouncer};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub enum Msg {
    RunNow,
    Reload(Config),
    SetRunning(bool),
    Changed,
    Shutdown,
}

/// Handle để giao tiếp với luồng nền.
pub struct Engine {
    tx: Sender<Msg>,
}

impl Engine {
    pub fn run_now(&self) {
        let _ = self.tx.send(Msg::RunNow);
    }
    pub fn reload(&self, cfg: Config) {
        let _ = self.tx.send(Msg::Reload(cfg));
    }
    pub fn set_running(&self, running: bool) {
        let _ = self.tx.send(Msg::SetRunning(running));
    }
    pub fn shutdown(&self) {
        let _ = self.tx.send(Msg::Shutdown);
    }

    pub fn start(
        app: AppHandle,
        cfg: Config,
        logger: Arc<Logger>,
        status: Arc<Mutex<Status>>,
    ) -> Engine {
        let (tx, rx) = mpsc::channel::<Msg>();
        let tx_watch = tx.clone();
        std::thread::spawn(move || {
            worker(app, rx, tx_watch, cfg, logger, status);
        });
        Engine { tx }
    }
}

fn worker(
    app: AppHandle,
    rx: Receiver<Msg>,
    tx_watch: Sender<Msg>,
    mut cfg: Config,
    logger: Arc<Logger>,
    status: Arc<Mutex<Status>>,
) {
    let mut running = cfg.running;
    let mut _debouncer: Option<Debouncer<notify::RecommendedWatcher>> = None;

    rebuild_watchers(&mut _debouncer, &cfg, &tx_watch, &logger, running);
    push_status(&app, &status, &cfg, running, "idle", None);
    logger.log("INFO", "Khởi động dịch vụ sao lưu.");

    // Chạy một lần ngay khi khởi động nếu đang bật.
    if running {
        run_all(&app, &cfg, &logger, &status, running);
    }

    loop {
        // Chế độ định kỳ + đang chạy -> chờ theo chu kỳ; ngược lại chờ tới khi có lệnh.
        let timeout = if running && cfg.mode == Mode::Periodic {
            Some(Duration::from_secs(cfg.interval_minutes.max(1) * 60))
        } else {
            None
        };

        let msg = match timeout {
            Some(t) => match rx.recv_timeout(t) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => break,
            },
        };

        match msg {
            // Hết chu kỳ (chế độ định kỳ) -> chạy sao lưu.
            None => run_all(&app, &cfg, &logger, &status, running),
            Some(Msg::RunNow) => run_all(&app, &cfg, &logger, &status, running),
            Some(Msg::Changed) => {
                if running && cfg.mode == Mode::Realtime {
                    run_all(&app, &cfg, &logger, &status, running);
                }
            }
            Some(Msg::SetRunning(b)) => {
                running = b;
                logger.log("INFO", if b { "Tiếp tục sao lưu." } else { "Tạm dừng sao lưu." });
                if b {
                    run_all(&app, &cfg, &logger, &status, running);
                } else {
                    push_status(&app, &status, &cfg, running, "idle", None);
                }
            }
            Some(Msg::Reload(new_cfg)) => {
                cfg = new_cfg;
                running = cfg.running;
                rebuild_watchers(&mut _debouncer, &cfg, &tx_watch, &logger, running);
                push_status(&app, &status, &cfg, running, "idle", None);
                if running {
                    run_all(&app, &cfg, &logger, &status, running);
                }
            }
            Some(Msg::Shutdown) => {
                logger.log("INFO", "Dừng dịch vụ sao lưu.");
                break;
            }
        }
    }
}

fn run_all(
    app: &AppHandle,
    cfg: &Config,
    logger: &Arc<Logger>,
    status: &Arc<Mutex<Status>>,
    running: bool,
) {
    let enabled: Vec<_> = cfg.pairs.iter().filter(|p| p.enabled).collect();
    if enabled.is_empty() {
        return;
    }
    push_status(app, status, cfg, running, "syncing", None);

    let mut total = backup::SyncResult::default();
    for pair in &enabled {
        let r = backup::sync_pair(pair, logger.as_ref());
        total.copied += r.copied;
        total.deleted += r.deleted;
        total.skipped += r.skipped;
        total.errors += r.errors;
        total.bytes += r.bytes;
    }

    let summary = format!(
        "Sao lưu xong: {} file copy, {} xóa, {} bỏ qua, {} lỗi ({:.1} MB).",
        total.copied,
        total.deleted,
        total.skipped,
        total.errors,
        total.bytes as f64 / 1_048_576.0
    );
    logger.log("INFO", &summary);
    logger.cleanup();
    push_status(app, status, cfg, running, "idle", Some(summary));
}

fn rebuild_watchers(
    slot: &mut Option<Debouncer<notify::RecommendedWatcher>>,
    cfg: &Config,
    tx: &Sender<Msg>,
    logger: &Arc<Logger>,
    running: bool,
) {
    // Bỏ watcher cũ.
    *slot = None;
    if !running || cfg.mode != Mode::Realtime {
        return;
    }
    let tx = tx.clone();
    let debouncer = new_debouncer(Duration::from_secs(3), move |res| {
        if let Ok(_events) = res {
            let _ = tx.send(Msg::Changed);
        }
    });
    let mut debouncer = match debouncer {
        Ok(d) => d,
        Err(e) => {
            logger.log("ERROR", &format!("Không khởi tạo được theo dõi thay đổi: {}", e));
            return;
        }
    };
    for pair in cfg.pairs.iter().filter(|p| p.enabled) {
        let path = std::path::Path::new(&pair.source);
        if path.is_dir() {
            if let Err(e) = debouncer.watcher().watch(path, RecursiveMode::Recursive) {
                logger.log("ERROR", &format!("Không theo dõi được {}: {}", pair.source, e));
            }
        }
    }
    *slot = Some(debouncer);
}

fn push_status(
    app: &AppHandle,
    status: &Arc<Mutex<Status>>,
    cfg: &Config,
    running: bool,
    activity: &str,
    last_summary: Option<String>,
) {
    let mut s = status.lock().unwrap();
    s.running = running;
    s.mode = match cfg.mode {
        Mode::Realtime => "realtime".into(),
        Mode::Periodic => "periodic".into(),
    };
    s.interval_minutes = cfg.interval_minutes;
    s.activity = activity.into();
    s.pairs = cfg.pairs.iter().filter(|p| p.enabled).count();
    if let Some(sum) = last_summary {
        s.last_summary = sum;
        s.last_run = Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    }
    let _ = app.emit("status", s.clone());
}
