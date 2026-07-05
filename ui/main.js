const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);

let config = null;

// ---------- Hiển thị trạng thái ----------
function renderStatus(s) {
  const dot = $("statusDot");
  const text = $("statusText");
  const toggle = $("btnToggle");

  if (s.activity === "syncing") {
    dot.className = "dot busy";
    text.textContent = "Đang sao lưu…";
  } else if (s.running) {
    dot.className = "dot on";
    const modeTxt =
      s.mode === "realtime"
        ? "thời gian thực"
        : `định kỳ ${s.interval_minutes} phút`;
    text.textContent = `Đang bật · ${modeTxt} · ${s.pairs} thư mục`;
  } else {
    dot.className = "dot off";
    text.textContent = "Đã tạm dừng";
  }

  toggle.textContent = s.running ? "Tạm dừng" : "Tiếp tục";

  if (s.last_run) {
    text.textContent += ` · Lần cuối: ${s.last_run}`;
  }
}

// ---------- Hiển thị danh sách cặp ----------
function renderPairs() {
  const list = $("pairList");
  const empty = $("pairEmpty");
  list.innerHTML = "";

  if (!config.pairs.length) {
    empty.style.display = "block";
    return;
  }
  empty.style.display = "none";

  for (const p of config.pairs) {
    const row = document.createElement("div");
    row.className = "pair" + (p.enabled ? "" : " disabled");
    row.innerHTML = `
      <div class="pair-info">
        <div class="pair-path" title="${escapeAttr(p.source)}">
          <b>Nguồn:</b> ${escapeHtml(p.source)}
        </div>
        <div class="pair-path" title="${escapeAttr(p.dest)}">
          <b>Đích:</b> ${escapeHtml(p.dest)}
        </div>
        <div class="pair-meta">
          <span class="badge ${p.mirror ? "mirror" : "keep"}">
            ${p.mirror ? "Mirror (xóa theo)" : "Giữ lại bản backup"}
          </span>
        </div>
      </div>
      <div class="pair-actions">
        <button class="btn btn-sm" data-act="toggle">${p.enabled ? "Tắt" : "Bật"}</button>
        <button class="btn btn-sm btn-danger" data-act="remove">Xóa</button>
      </div>`;

    row
      .querySelector('[data-act="toggle"]')
      .addEventListener("click", async () => {
        config = await invoke("toggle_pair", { id: p.id, enabled: !p.enabled });
        renderPairs();
      });
    row
      .querySelector('[data-act="remove"]')
      .addEventListener("click", async () => {
        if (confirm(`Xóa cặp sao lưu này?\n${p.source}`)) {
          config = await invoke("remove_pair", { id: p.id });
          renderPairs();
        }
      });
    list.appendChild(row);
  }
}

function escapeHtml(s) {
  return s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c]));
}
function escapeAttr(s) {
  return escapeHtml(s).replace(/"/g, "&quot;");
}

// ---------- Chế độ & cài đặt ----------
function renderControls() {
  document
    .querySelectorAll('input[name="mode"]')
    .forEach((r) => (r.checked = r.value === config.mode));
  $("interval").value = config.interval_minutes;
  $("autostart").checked = config.autostart;
}

async function applyMode() {
  const mode = document.querySelector('input[name="mode"]:checked').value;
  const interval = Math.max(1, parseInt($("interval").value || "30", 10));
  config = await invoke("set_mode", { mode, intervalMinutes: interval });
}

// ---------- Log ----------
async function loadLogs() {
  const lines = await invoke("get_logs");
  const view = $("logView");
  view.textContent = lines.join("\n");
  view.scrollTop = view.scrollHeight;
}

function appendLog(line) {
  const view = $("logView");
  const atBottom = view.scrollTop + view.clientHeight >= view.scrollHeight - 20;
  view.textContent += (view.textContent ? "\n" : "") + line;
  if (atBottom) view.scrollTop = view.scrollHeight;
}

// ---------- Hộp thoại thêm cặp ----------
function openDialog() {
  $("dlgSource").value = "";
  $("dlgDest").value = "";
  $("dlgMirror").checked = false;
  $("dialog").classList.remove("hidden");
}
function closeDialog() {
  $("dialog").classList.add("hidden");
}

// ---------- Sự kiện ----------
function wireEvents() {
  $("btnAddPair").addEventListener("click", openDialog);
  $("dlgCancel").addEventListener("click", closeDialog);

  $("dlgPickSource").addEventListener("click", async () => {
    const p = await invoke("pick_folder");
    if (p) $("dlgSource").value = p;
  });
  $("dlgPickDest").addEventListener("click", async () => {
    const p = await invoke("pick_folder");
    if (p) $("dlgDest").value = p;
  });

  $("dlgSave").addEventListener("click", async () => {
    const source = $("dlgSource").value.trim();
    const dest = $("dlgDest").value.trim();
    if (!source || !dest) {
      alert("Vui lòng chọn cả thư mục nguồn và thư mục đích.");
      return;
    }
    if (source === dest) {
      alert("Thư mục nguồn và đích không được trùng nhau.");
      return;
    }
    config = await invoke("add_pair", {
      source,
      dest,
      mirror: $("dlgMirror").checked,
    });
    closeDialog();
    renderPairs();
  });

  $("btnToggle").addEventListener("click", async () => {
    config = await invoke("set_running", { running: !config.running });
  });

  $("btnBackupNow").addEventListener("click", async () => {
    await invoke("backup_now");
  });

  $("btnRefreshLog").addEventListener("click", loadLogs);

  document
    .querySelectorAll('input[name="mode"]')
    .forEach((r) => r.addEventListener("change", applyMode));
  $("interval").addEventListener("change", applyMode);

  $("autostart").addEventListener("change", async () => {
    config = await invoke("set_autostart", { enabled: $("autostart").checked });
  });
}

// ---------- Khởi tạo ----------
async function init() {
  config = await invoke("get_config");
  renderPairs();
  renderControls();

  const status = await invoke("get_status");
  renderStatus(status);

  await loadLogs();
  wireEvents();

  await listen("status", (e) => renderStatus(e.payload));
  await listen("log", (e) => appendLog(e.payload));
}

init();
