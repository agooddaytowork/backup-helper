(function () {
  const { invoke } = window.__TAURI__.core;
  const $ = (id) => document.getElementById(id);
  const esc = (s) =>
    String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

  const { listen } = window.__TAURI__.event;

  let cfg = { pairs: [] };
  let connMap = {}; // pair_id -> connected (từ v2_conn_status + event)

  // ---------- Chuyển giữa chế độ đơn giản <-> nâng cao ----------
  function showAdvanced(on) {
    $("tab-backup").classList.toggle("hidden", on);
    $("tab-v2").classList.toggle("hidden", !on);
  }
  const advBtn = $("btnAdvanced");
  if (advBtn) advBtn.addEventListener("click", () => showAdvanced(true));
  const backBtn = $("btnBackSimple");
  if (backBtn) backBtn.addEventListener("click", () => showAdvanced(false));

  // ---------- Format helpers ----------
  function formatOp(op) {
    const k = op.kind;
    if (k && typeof k === "object" && "Copy" in k) {
      const fwd = k.Copy === "OriginToWorking";
      return {
        cls: "v2-copy",
        badge: "Copy",
        label: fwd ? "Gốc → Bản làm việc" : "Bản làm việc → Gốc",
      };
    }
    if (k && typeof k === "object" && "Delete" in k) {
      return {
        cls: "v2-del",
        badge: "Xoá",
        label: k.Delete === "Working" ? "Xoá ở Bản làm việc" : "Xoá ở Gốc",
      };
    }
    return { cls: "", badge: "?", label: "" };
  }

  function formatConflict(c) {
    const k = c.kind;
    if (typeof k === "string") {
      if (k === "BothModified") return "Cả 2 bên cùng sửa";
      if (k === "BothCreated") return "Cả 2 bên cùng tạo mới";
    } else if (k && "EditVsDelete" in k) {
      return k.EditVsDelete.deleted === "Origin"
        ? "Gốc xoá / Bản làm việc sửa"
        : "Bản làm việc xoá / Gốc sửa";
    }
    return "Xung đột";
  }

  // ---------- Render pairs ----------
  function renderPairs() {
    const list = $("v2PairList");
    list.innerHTML = "";
    $("v2PairEmpty").style.display = cfg.pairs.length ? "none" : "block";
    for (const p of cfg.pairs) {
      const row = document.createElement("div");
      row.className = "pair";
      row.innerHTML = `
        <div class="pair-info">
          <div class="pair-path"><b>${esc(p.name)}</b>
            <span style="font-size:11.5px;font-weight:600;margin-left:6px;color:${
              connMap[p.id] === false ? "var(--red)" : "var(--green)"
            }">${connMap[p.id] === false ? "● Mất kết nối" : "● Đang kết nối"}</span></div>
          <div class="pair-path" title="${esc(p.origin)}">Gốc: ${esc(p.origin)}</div>
          <div class="pair-path" title="${esc(p.working)}">Làm việc: ${esc(p.working)}</div>
        </div>
        <div class="pair-actions">
          <button class="btn btn-sm" data-a="plan">Kiểm tra</button>
          <button class="btn btn-sm btn-primary" data-a="apply">Đồng bộ</button>
          <button class="btn btn-sm btn-danger" data-a="del">Xoá</button>
        </div>`;
      row.querySelector('[data-a="plan"]').onclick = () => doPlan(p.id, p.name, true);
      row.querySelector('[data-a="apply"]').onclick = () => doApply(p.id, p.name);
      row.querySelector('[data-a="del"]').onclick = async () => {
        if (confirm(`Xoá cặp “${p.name}”? (không xoá file, chỉ bỏ khỏi danh sách)`)) {
          cfg = await invoke("v2_remove_pair", { id: p.id });
          renderPairs();
        }
      };
      list.appendChild(row);
    }
  }

  // ---------- Plan / Apply / Resolve ----------
  function showResult(title) {
    $("v2ResultCard").style.display = "block";
    $("v2ResultTitle").textContent = title;
    return $("v2Result");
  }

  function renderOpsAndConflicts(box, pairId, name, ops, conflicts, withApprove) {
    let html = "";
    if (ops.length) {
      html += `<div class="v2-sub">Thay đổi sẽ áp dụng (${ops.length})</div>`;
      for (const op of ops) {
        const f = formatOp(op);
        html += `<div class="v2-line"><span class="v2-badge ${f.cls}">${f.badge}</span>
          <span class="v2-path" title="${esc(op.rel_path)}">${esc(op.rel_path)}</span>
          <span style="color:var(--muted);font-size:12px">${f.label}</span></div>`;
      }
    }
    if (conflicts.length) {
      html += `<div class="v2-sub" style="color:var(--amber)">Xung đột cần bạn quyết định (${conflicts.length})</div>`;
      html += `<div id="v2Conflicts"></div>`;
    }
    if (!ops.length && !conflicts.length) {
      html += `<p class="empty" style="text-align:left">Không có thay đổi — đã đồng bộ.</p>`;
    }
    if (withApprove && ops.length) {
      html += `<div style="margin-top:12px;display:flex;gap:8px">
        <button class="btn btn-primary" id="v2Approve">✓ Duyệt &amp; Đồng bộ (${ops.length} thay đổi)</button>
        <button class="btn" id="v2Dismiss">Bỏ qua</button></div>`;
    }
    box.innerHTML = html;

    if (withApprove && ops.length) {
      $("v2Approve").onclick = () => doApply(pairId, name);
      $("v2Dismiss").onclick = () => { $("v2ResultCard").style.display = "none"; };
    }

    if (conflicts.length) {
      const cbox = $("v2Conflicts");
      for (const c of conflicts) {
        const row = document.createElement("div");
        row.className = "v2-conflict-row";
        row.innerHTML = `
          <div style="min-width:0">
            <div class="v2-path" title="${esc(c.rel_path)}">${esc(c.rel_path)}</div>
            <div style="font-size:12px;color:var(--amber)">${formatConflict(c)}</div>
          </div>
          <div class="acts">
            <button class="btn btn-sm" data-k="origin">Giữ Gốc</button>
            <button class="btn btn-sm" data-k="working">Giữ Bản làm việc</button>
          </div>`;
        row.querySelector('[data-k="origin"]').onclick = () => resolve(pairId, name, c.rel_path, "origin");
        row.querySelector('[data-k="working"]').onclick = () => resolve(pairId, name, c.rel_path, "working");
        cbox.appendChild(row);
      }
    }
  }

  async function doPlan(pairId, name, withApprove, title) {
    const box = showResult(title || `Kiểm tra: ${name}`);
    box.innerHTML = "<p class='empty'>Đang quét…</p>";
    try {
      const plan = await invoke("v2_plan", { id: pairId });
      renderOpsAndConflicts(box, pairId, name, plan.ops, plan.conflicts, withApprove);
    } catch (e) {
      box.innerHTML = `<p class="empty" style="color:var(--red)">Lỗi: ${esc(e)}</p>`;
    }
  }

  async function doApply(pairId, name) {
    const box = showResult(`Đồng bộ: ${name}`);
    box.innerHTML = "<p class='empty'>Đang đồng bộ…</p>";
    try {
      const r = await invoke("v2_apply", { id: pairId });
      let html = `<div class="v2-line"><span class="v2-badge v2-ok">Xong</span>
        <span>Copy: ${r.copied} · Xoá: ${r.deleted} · Xung đột: ${r.conflicts.length}</span></div>`;
      box.innerHTML = html;
      if (r.conflicts.length) {
        box.innerHTML += `<div class="v2-sub" style="color:var(--amber)">Xung đột cần quyết định (${r.conflicts.length})</div><div id="v2Conflicts"></div>`;
        const cbox = $("v2Conflicts");
        for (const c of r.conflicts) {
          const row = document.createElement("div");
          row.className = "v2-conflict-row";
          row.innerHTML = `
            <div style="min-width:0"><div class="v2-path" title="${esc(c.rel_path)}">${esc(c.rel_path)}</div>
              <div style="font-size:12px;color:var(--amber)">${formatConflict(c)}</div></div>
            <div class="acts">
              <button class="btn btn-sm" data-k="origin">Giữ Gốc</button>
              <button class="btn btn-sm" data-k="working">Giữ Bản làm việc</button></div>`;
          row.querySelector('[data-k="origin"]').onclick = () => resolve(pairId, name, c.rel_path, "origin");
          row.querySelector('[data-k="working"]').onclick = () => resolve(pairId, name, c.rel_path, "working");
          cbox.appendChild(row);
        }
      }
    } catch (e) {
      box.innerHTML = `<p class="empty" style="color:var(--red)">Lỗi: ${esc(e)}</p>`;
    }
  }

  async function resolve(pairId, name, rel, keep) {
    try {
      await invoke("v2_resolve", { id: pairId, rel, keep });
      await doPlan(pairId, name, true); // làm mới sau khi xử lý
    } catch (e) {
      alert("Lỗi xử lý xung đột: " + e);
    }
  }

  // ---------- Trạng thái kết nối + event từ watcher ----------
  async function loadConn() {
    try {
      const list = await invoke("v2_conn_status");
      connMap = {};
      for (const s of list) connMap[s.pair_id] = s.connected;
      renderPairs();
    } catch (e) { /* chưa có pair nào cũng không sao */ }
  }

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
    listen("v2-reconnected", (e) => {
      showAdvanced(true);
      const id = e.payload.pair_id;
      doPlan(id, pairName(id), true, `Kết nối lại: ${pairName(id)} — xem & duyệt thay đổi`);
    });
    // Định kỳ gặp conflict: mở thẻ duyệt để xử lý từng file.
    listen("v2-conflicts", (e) => {
      showAdvanced(true);
      doPlan(e.payload.pair_id, pairName(e.payload.pair_id), true);
    });
  }

  // ---------- Modals ----------
  function wirePairDialog() {
    $("v2AddPair").onclick = () => {
      $("v2pName").value = "";
      $("v2pOrigin").value = "";
      $("v2pWorking").value = "";
      $("v2PairDlg").classList.remove("hidden");
    };
    $("v2pCancel").onclick = () => $("v2PairDlg").classList.add("hidden");
    $("v2pPickOrigin").onclick = async () => {
      const p = await invoke("pick_folder");
      if (p) $("v2pOrigin").value = p;
    };
    $("v2pPickWorking").onclick = async () => {
      const p = await invoke("pick_folder");
      if (p) $("v2pWorking").value = p;
    };
    $("v2pSave").onclick = async () => {
      const origin = $("v2pOrigin").value.trim();
      const working = $("v2pWorking").value.trim();
      if (!origin || !working) return alert("Chọn cả 2 thư mục.");
      try {
        cfg = await invoke("v2_add_pair", { name: $("v2pName").value.trim(), origin, working });
        $("v2PairDlg").classList.add("hidden");
        renderPairs();
      } catch (e) {
        alert("Lỗi: " + e);
      }
    };
  }

  function renderAuto() {
    $("v2Auto").checked = !!cfg.auto;
    $("v2Interval").value = cfg.interval_minutes || 30;
  }
  async function saveAuto() {
    const interval = Math.max(1, parseInt($("v2Interval").value || "30", 10));
    cfg = await invoke("v2_set_auto", { auto: $("v2Auto").checked, intervalMinutes: interval });
  }
  function wireAuto() {
    $("v2Auto").addEventListener("change", saveAuto);
    $("v2Interval").addEventListener("change", saveAuto);
  }

  function wireUndo() {
    $("v2UndoLast").onclick = async () => {
      if (!confirm("Hoàn tác lần đồng bộ gần nhất? Nội dung file sẽ được khôi phục về trước.")) return;
      try {
        const run = await invoke("v2_undo_last");
        alert("Đã hoàn tác: " + run);
      } catch (e) {
        alert("Không hoàn tác được: " + e);
      }
    };
  }

  // ---------- Init ----------
  async function init() {
    try {
      cfg = await invoke("v2_get_config");
    } catch (e) {
      cfg = { pairs: [] };
    }
    renderPairs();
    renderAuto();
    wirePairDialog();
    wireAuto();
    wireUndo();
    wireEvents();
    loadConn();
  }

  init();
})();
