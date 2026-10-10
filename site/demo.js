// The interactive app replica and the install tabs. Mirrors the real UI in src/main.rs.
(() => {
  const CATS = ["all", "text", "link", "media", "file"];
  const GROUP = { txt: 1, code: 1, url: 2, img: 3, vid: 3, aud: 3, pdf: 4, file: 4 };
  const items = [
    { t: "url", x: "https://github.com/erfjab/CoBox", c: "14:32", src: "firefox", day: "pinned" },
    { t: "code", x: "cargo build --profile dist", c: "09:12", src: "Code", day: "pinned", lines: ["cargo build --profile dist"] },
    { t: "txt", x: "Meeting moved to Thursday 3pm, same room.", c: "14:28", src: "Slack", day: "today" },
    { t: "img", x: "image 1920×1080", c: "14:20", src: "Snipping Tool", day: "today", thumb: true },
    { t: "code", x: "fn main() {", c: "13:55", src: "Code", day: "today", lines: ["fn main() {", "    println!(\"hello, cobox\");", "}"] },
    { t: "txt", x: "سلام! فردا ساعت ۱۰ جلسه داریم.", c: "12:41", src: "Telegram", day: "today", rtl: true },
    { t: "url", x: "https://doc.rust-lang.org/std/sync/", c: "11:03", src: "chrome", day: "today" },
    { t: "pdf", x: "invoice-2026-10.pdf", c: "10:17", src: "explorer", day: "today", thumb: true },
    { t: "txt", x: "ssh deploy@203.0.113.7", c: "18:46", src: "Terminal", day: "yesterday" },
    { t: "vid", x: "demo-recording.mp4", c: "16:02", src: "explorer", day: "yesterday", thumb: true },
    { t: "file", x: "report.xlsx  notes.md  logo.svg", c: "15:30", src: "explorer", day: "yesterday", meta: "3 files" },
    { t: "txt", x: "Thanks, that fixed it!", c: "11:12", src: "Mail", day: "monday" },
  ];
  let q = "", cat = 0, sel = 0, toast = null, timer;

  const $ = (id) => document.getElementById(id);
  const app = $("app"), list = $("list"), preview = $("preview");
  const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

  const view = () => {
    const words = q.toLowerCase().split(/\s+/).filter(Boolean);
    return items.filter((i) => (cat === 0 || GROUP[i.t] === cat)
      && words.every((w) => i.x.toLowerCase().includes(w) || i.src.toLowerCase().includes(w)));
  };

  function render() {
    const v = view();
    sel = Math.min(sel, Math.max(0, v.length - 1));
    $("q").textContent = q;
    $("ph").style.display = q ? "none" : "";
    $("cats").innerHTML = CATS.map((n, i) => `<span data-cat="${i}" class="${i === cat ? "on" : ""}">${i === cat ? `[${n}]` : n}</span>`).join("");

    const word = q.trim().split(/\s+/)[0]?.toLowerCase() || "";
    let html = "", last = "", selRow = 0, row = 0;
    v.forEach((it, vi) => {
      if (it.day !== last) { html += `<div class="head ${it.day === "pinned" ? "pin" : ""}">──<b>${it.day}</b></div>`; last = it.day; row++; }
      let text = esc(it.x);
      const at = word ? it.x.toLowerCase().indexOf(word) : -1;
      if (at >= 0) text = esc(it.x.slice(0, at)) + "<mark>" + esc(it.x.slice(at, at + word.length)) + "</mark>" + esc(it.x.slice(at + word.length));
      if (vi === sel) selRow = row;
      html += `<div class="row ${vi === sel ? "on" : ""}" data-vi="${vi}"><span class="t">${it.t}</span><span class="x ${it.rtl ? "rtl" : ""}">${text}</span><span class="c">${it.c}</span></div>`;
      row++;
    });
    list.innerHTML = html || "";
    // Keep the selected row in view, like the app's one-row-at-a-time scrolling.
    const rowH = list.firstElementChild?.offsetHeight || 28;
    const visible = Math.floor(list.clientHeight / rowH);
    list.scrollTop = Math.max(0, (selRow + 1 - visible) * rowH);

    const it = v[sel];
    if (!it) {
      preview.innerHTML = `<div class="meta" style="color:var(--faint)">nothing matches · try another word</div>`;
    } else {
      const isText = ["txt", "code", "url"].includes(it.t);
      const lines = it.lines || [it.x];
      const meta = [it.t, it.src, it.day === "pinned" ? "pinned" : it.day, it.c].join(" · ") + (isText ? ` · ${lines.length} lines` : "");
      preview.innerHTML = `<div class="meta">${esc(meta)}</div><div class="body">${it.thumb ? '<div class="thumb"></div>' : ""}`
        + `<div class="lines ${it.rtl ? "rtl" : ""}" style="${isText ? "" : "color:var(--dim)"}">${esc(lines.join("\n"))}</div></div>`;
    }
    $("count").innerHTML = toast ? `<span class="toast">${toast}</span>` : `${v.length} items`;
  }

  function flash(msg) {
    toast = msg; render();
    clearTimeout(timer);
    timer = setTimeout(() => { toast = null; render(); }, 1600);
  }

  app.addEventListener("keydown", (e) => {
    const v = view();
    if (e.key === "ArrowDown") sel = Math.min(sel + 1, v.length - 1);
    else if (e.key === "ArrowUp") sel = Math.max(sel - 1, 0);
    else if (e.key === "Tab") { cat = (cat + (e.shiftKey ? CATS.length - 1 : 1)) % CATS.length; sel = 0; }
    else if (e.key === "Enter") { if (v[sel]) flash(e.shiftKey ? "pasted as plain text" : "pasted"); return e.preventDefault(); }
    else if (e.key === "Escape") { if (q) { q = ""; } else { app.blur(); } }
    else if (e.key === "Backspace") { q = e.ctrlKey ? "" : q.slice(0, -1); }
    else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) { q += e.key; sel = 0; }
    else return;
    e.preventDefault();
    render();
  });
  app.addEventListener("click", (e) => {
    app.focus({ preventScroll: true });
    const c = e.target.closest("[data-cat]"), r = e.target.closest("[data-vi]");
    if (c) { cat = +c.dataset.cat; sel = 0; }
    if (r) { sel = +r.dataset.vi; if (e.detail >= 2) flash("pasted"); }
    render();
  });

  // Until someone touches it, the demo types a search by itself.
  let auto = true;
  const stopAuto = () => { auto = false; };
  app.addEventListener("pointerdown", stopAuto);
  app.addEventListener("keydown", stopAuto, true);
  const script = ["rust", "", "github", ""];
  async function play() {
    const wait = (ms) => new Promise((r) => setTimeout(r, ms));
    for (let n = 0; auto; n++) {
      const word = script[n % script.length];
      await wait(1800);
      if (!auto) return;
      if (word) {
        for (const ch of word) { if (!auto) return; q += ch; sel = 0; render(); await wait(140); }
        await wait(1200);
        if (!auto) return;
        flash("pasted");
        await wait(900);
        while (q && auto) { q = q.slice(0, -1); render(); await wait(50); }
      } else {
        for (let i = 0; i < 4 && auto; i++) { sel++; render(); await wait(380); }
        for (let i = 0; i < 4 && auto; i++) { sel = Math.max(0, sel - 1); render(); await wait(200); }
      }
    }
  }
  render();
  if (!matchMedia("(prefers-reduced-motion: reduce)").matches) {
    new IntersectionObserver((es, o) => { if (es[0].isIntersecting) { o.disconnect(); play(); } }, { threshold: 0.4 }).observe(app);
  }

  // ── install tabs ──
  const OS = {
    windows: { shell: "powershell", cmd: "irm https://raw.githubusercontent.com/erfjab/CoBox/master/install.ps1 | iex", note: "Installs to %LOCALAPPDATA%\\Programs\\CoBox and starts with Windows." },
    macos: { shell: "terminal", cmd: "curl -fsSL https://raw.githubusercontent.com/erfjab/CoBox/master/install.sh | sh", note: "Installs CoBox.app. To paste with Enter, allow CoBox in System Settings › Privacy & Security › Accessibility." },
    linux: { shell: "terminal", cmd: "curl -fsSL https://raw.githubusercontent.com/erfjab/CoBox/master/install.sh | sh", note: "Installs to ~/.local/bin. On Wayland, bind a keyboard shortcut to the command cobox to open it." },
  };
  const tabs = document.querySelectorAll(".tabs button");
  const pick = (os) => {
    tabs.forEach((b) => b.setAttribute("aria-selected", String(b.dataset.os === os)));
    $("shell").textContent = OS[os].shell;
    $("cmd").textContent = OS[os].cmd;
    $("os-note").textContent = OS[os].note;
  };
  tabs.forEach((b) => b.addEventListener("click", () => pick(b.dataset.os)));
  const ua = navigator.userAgent;
  pick(/Mac/.test(ua) ? "macos" : /Linux|X11/.test(ua) && !/Android/.test(ua) ? "linux" : "windows");
  $("copy").addEventListener("click", async () => {
    try { await navigator.clipboard.writeText($("cmd").textContent); $("copy").textContent = "copied"; }
    catch { $("copy").textContent = "select it"; }
    setTimeout(() => { $("copy").textContent = "copy"; }, 1400);
  });
})();
