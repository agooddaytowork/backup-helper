(function () {
  const { invoke } = window.__TAURI__.core;
  const $ = (id) => document.getElementById(id);
  const esc = (s) =>
    String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

  let cfg = { pairs: [], targets: [] };
  let remotes = [];
  let cloudLoaded = false;

  // ---------- Chuyển giữa chế độ đơn giản <-> nâng cao ----------
  function showAdvanced(on) {
    $("tab-backup").classList.toggle("hidden", on);
    $("tab-v2").classList.toggle("hidden", !on);
    if (on && !cloudLoaded) loadCloud();
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
          <div class="pair-path"><b>${esc(p.name)}</b></div>
          <div class="pair-path" title="${esc(p.origin)}">Gốc: ${esc(p.origin)}</div>
          <div class="pair-path" title="${esc(p.working)}">Làm việc: ${esc(p.working)}</div>
        </div>
        <div class="pair-actions">
          <button class="btn btn-sm" data-a="plan">Kiểm tra</button>
          <button class="btn btn-sm btn-primary" data-a="apply">Đồng bộ</button>
          <button class="btn btn-sm" data-a="repl">Đẩy cloud</button>
          <button class="btn btn-sm btn-danger" data-a="del">Xoá</button>
        </div>`;
      row.querySelector('[data-a="plan"]').onclick = () => doPlan(p.id, p.name);
      row.querySelector('[data-a="apply"]').onclick = () => doApply(p.id, p.name);
      row.querySelector('[data-a="repl"]').onclick = () => doReplicate(p.id, p.name);
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

  function renderOpsAndConflicts(box, pairId, name, ops, conflicts) {
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
    box.innerHTML = html;

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

  async function doPlan(pairId, name) {
    const box = showResult(`Kiểm tra: ${name}`);
    box.innerHTML = "<p class='empty'>Đang quét…</p>";
    try {
      const plan = await invoke("v2_plan", { id: pairId });
      renderOpsAndConflicts(box, pairId, name, plan.ops, plan.conflicts);
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
      if (r.replication && r.replication.length) {
        html += `<div class="v2-sub">Nhân bản cloud</div>`;
        for (const s of r.replication) {
          html += `<div class="v2-line"><span class="v2-badge ${s.ok ? "v2-ok" : "v2-err"}">${
            s.ok ? "OK" : "Lỗi"
          }</span><span>${esc(s.target_name)}: ${esc(s.message)}</span></div>`;
        }
      }
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
      await doPlan(pairId, name); // làm mới sau khi xử lý
    } catch (e) {
      alert("Lỗi xử lý xung đột: " + e);
    }
  }

  async function doReplicate(pairId, name) {
    const box = showResult(`Đẩy cloud: ${name}`);
    box.innerHTML = "<p class='empty'>Đang đẩy lên cloud…</p>";
    try {
      const list = await invoke("v2_replicate", { id: pairId });
      if (!list.length) {
        box.innerHTML = "<p class='empty' style='text-align:left'>Chưa có đích cloud nào (hoặc rclone chưa sẵn sàng).</p>";
        return;
      }
      let html = `<div class="v2-sub">Kết quả nhân bản</div>`;
      for (const s of list) {
        html += `<div class="v2-line"><span class="v2-badge ${s.ok ? "v2-ok" : "v2-err"}">${
          s.ok ? "OK" : "Lỗi"
        }</span><span>${esc(s.target_name)}: ${esc(s.message)}</span></div>`;
      }
      box.innerHTML = html;
    } catch (e) {
      box.innerHTML = `<p class="empty" style="color:var(--red)">Lỗi: ${esc(e)}</p>`;
    }
  }

  // ---------- Cloud ----------
  async function loadCloud() {
    cloudLoaded = true;
    const el = $("v2CloudStatus");
    el.textContent = "Đang kiểm tra…";
    try {
      const st = await invoke("v2_status");
      remotes = st.remotes || [];
      if (!st.rclone_installed) {
        el.innerHTML = `<span style="color:var(--red)">Chưa tìm thấy rclone trên máy.</span> Cài rclone rồi bấm “Làm mới”.`;
      } else {
        el.innerHTML = `<span style="color:var(--green)">rclone OK</span> — ${esc(st.rclone_version)}<br />
          Remote đã kết nối: ${remotes.length ? remotes.map(esc).join(", ") : "(chưa có)"}`;
      }
    } catch (e) {
      el.innerHTML = `<span style="color:var(--red)">Lỗi: ${esc(e)}</span>`;
    }
    renderTargets();
  }

  function renderTargets() {
    const list = $("v2TargetList");
    list.innerHTML = "";
    $("v2TargetEmpty").style.display = cfg.targets.length ? "none" : "block";
    for (const t of cfg.targets) {
      const row = document.createElement("div");
      row.className = "pair";
      row.innerHTML = `
        <div class="pair-info">
          <div class="pair-path"><b>${esc(t.name)}</b></div>
          <div class="pair-path">${esc(t.remote)}${esc(t.dest_path)}</div>
          <div class="pair-meta"><span class="badge ${t.mirror ? "mirror" : "keep"}">${
            t.mirror ? "Mirror" : "Chỉ thêm/cập nhật"
          }</span></div>
        </div>
        <div class="pair-actions">
          <button class="btn btn-sm btn-danger" data-a="del">Xoá</button>
        </div>`;
      row.querySelector('[data-a="del"]').onclick = async () => {
        if (confirm(`Xoá đích cloud “${t.name}”?`)) {
          cfg = await invoke("v2_remove_target", { id: t.id });
          renderTargets();
        }
      };
      list.appendChild(row);
    }
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

  function wireTargetDialog() {
    $("v2AddTarget").onclick = () => {
      if (!remotes.length) {
        return alert("Chưa có remote cloud nào. Hãy kết nối Google Drive/OneDrive trước.");
      }
      const sel = $("v2tRemote");
      sel.innerHTML = remotes.map((r) => `<option value="${esc(r)}">${esc(r)}</option>`).join("");
      $("v2tName").value = "";
      $("v2tDest").value = "";
      $("v2tMirror").checked = false;
      $("v2TargetDlg").classList.remove("hidden");
    };
    $("v2tCancel").onclick = () => $("v2TargetDlg").classList.add("hidden");
    $("v2tSave").onclick = async () => {
      const name = $("v2tName").value.trim() || $("v2tRemote").value;
      const remote = $("v2tRemote").value;
      const dest_path = $("v2tDest").value.trim();
      cfg = await invoke("v2_add_target", { name, remote, destPath: dest_path, mirror: $("v2tMirror").checked });
      $("v2TargetDlg").classList.add("hidden");
      renderTargets();
    };
  }

  function wireCloud() {
    $("v2RefreshCloud").onclick = loadCloud;
    document.querySelectorAll(".v2Connect").forEach((b) =>
      b.addEventListener("click", async () => {
        const provider = b.dataset.provider;
        const base = provider === "drive" ? "gdrive" : "onedrive";
        // Tự đặt tên, tránh trùng remote đã có (prompt() không chạy trong webview).
        let name = base;
        let i = 2;
        while (remotes.includes(name + ":")) {
          name = base + i;
          i++;
        }
        const el = $("v2CloudStatus");
        el.innerHTML = `<span style="color:var(--amber)">Đang mở trình duyệt để đăng nhập “${esc(
          name
        )}”… Đăng nhập &amp; cấp quyền xong, cửa sổ này sẽ tự cập nhật.</span>`;
        try {
          const msg = await invoke("v2_connect_remote", { name, provider });
          el.innerHTML = `<span style="color:var(--green)">${esc(msg)}</span>`;
        } catch (e) {
          el.innerHTML = `<span style="color:var(--red)">Kết nối thất bại: ${esc(e)}</span>`;
        }
        loadCloud();
      })
    );
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
      cfg = { pairs: [], targets: [] };
    }
    renderPairs();
    renderTargets();
    renderAuto();
    wirePairDialog();
    wireTargetDialog();
    wireCloud();
    wireAuto();
    wireUndo();
  }

  init();
})();
