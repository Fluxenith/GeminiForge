// GeminiForge content script for gemini.google.com
// Watches Gemini's replies for {"gf_tool": ...} blocks, runs them through the
// local server (with the user's approval), and sends the results back.
(() => {
  if (window.__geminiForge) return;
  window.__geminiForge = true;

  // ------------------------------------------------------------ settings
  const READ_ONLY = new Set(["cd", "list_dir", "read_file", "search", "find_files", "screenshot", "wait",
    "clipboard_get", "list_windows", "ui_elements", "zoom"]);
  const MAX_AUTO_STEPS = 40;      // ask before continuing after this many rounds
  const STABLE_TICKS = 2;         // reply must stop changing for this many seconds
  const hasPrompt = (text) => /GeminiForge\s*(?:tool mode|tools\]|ツールモード|도구 모드|工具模式)/i.test(text);
  const MODES = ["manual", "readonly", "auto"];
  let language = geminiForgeLanguage(chrome.i18n.getUILanguage());
  let messages = {};
  function t(key, values = {}) {
    const template = messages[key]?.message || chrome.i18n.getMessage(key);
    return template.replace(/\{(\w+)\}/g, (match, name) => values[name] ?? match);
  }

  const state = { enabled: true, mode: "readonly", autoPrompt: true, busy: false, steps: 0, abort: false };
  let allowed = new Set();         // tools the user allowed for the current chat
  const seen = new WeakSet();
  const handled = new Set();
  const track = new WeakMap();
  let ignoreUntil = Date.now() + 4000; // existing chat history is never executed
  let lastUrl = location.href;
  async function applyLanguage(selected) {
    language = selected;
    const response = await bg({ type: "messages", language: selected });
    if (language !== selected) return;
    messages = response.messages || {};
    render();
  }

  const save = () => chrome.storage.local.set({ enabled: state.enabled, mode: state.mode, autoPrompt: state.autoPrompt });

  // ------------------------------------------------------------ utils
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const $ = (s, root = document) => root.querySelector(s);
  const $$ = (s, root = document) => [...root.querySelectorAll(s)];
  const hash = (s) => { let h = 5381; for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0; return h; };
  const bg = (msg) => new Promise((resolve) => {
    try {
      chrome.runtime.sendMessage(msg, (r) => {
        const error = chrome.runtime.lastError?.message;
        if (r?.offline) r.error = t("offline") + " " + r.error;
        resolve(r || { ok: false, error: error || t("noResponse") });
      });
    } catch (e) {
      resolve({ ok: false, error: t("updated") });
    }
  });
  async function waitFor(fn, timeout = 5000, every = 150) {
    const end = Date.now() + timeout;
    while (Date.now() < end) {
      const v = fn();
      if (v) return v;
      await sleep(every);
    }
    return null;
  }

  // ------------------------------------------------------------ Gemini page
  const editor = () => $("rich-textarea .ql-editor") || $('div[contenteditable="true"][role="textbox"]');
  const responses = () => $$("model-response");
  const isGenerating = () =>
    $$("button").some((b) => /停止|中止|중지|중단|stop/i.test(b.getAttribute("aria-label") || "") && b.offsetParent !== null);

  function sendButton() {
    return $('button[aria-label="傳送訊息"]') || $('button[aria-label*="Send message"]') || $("button.send-button") ||
      $$("input-area-v2 button, input-container button").find((b) => /傳送|发送|送信|보내기|전송|send/i.test(b.getAttribute("aria-label") || ""));
  }
  function uploadMenuButton() {
    return $('button[aria-label="上傳與工具"]') ||
      $$("input-area-v2 button, input-container button").find((b) => /上傳|上传|アップロード|업로드|upload/i.test(b.getAttribute("aria-label") || ""));
  }

  function insertText(text, atStart = false) {
    const ed = editor();
    if (!ed) throw new Error(t("noEditor"));
    ed.focus();
    const sel = window.getSelection();
    const range = document.createRange();
    range.selectNodeContents(ed);
    range.collapse(atStart);
    sel.removeAllRanges();
    sel.addRange(range);
    document.execCommand("insertText", false, text);
  }

  class UploadConsentError extends Error {}

  async function uploadImage(b64) {
    const bin = atob(b64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const file = new File([bytes], `screenshot_${Date.now()}.jpg`, { type: "image/jpeg" });

    const before = $$("uploader-file-preview").length;
    const btn = uploadMenuButton();
    if (!btn) throw new Error(t("noUpload"));
    btn.click();
    const input = await waitFor(() => $$('input[type="file"]').find((i) => /image/.test(i.accept)), 4000);
    if (!input) { closeOverlays(); throw new Error(t("noImageInput")); }
    const dt = new DataTransfer();
    dt.items.add(file);
    input.files = dt.files;
    input.dispatchEvent(new Event("change", { bubbles: true }));
    await sleep(300);
    if (consentDialogOpen()) {
      throw new UploadConsentError(t("consent"));
    }
    closeOverlays();
    const ok = await waitFor(() => $$("uploader-file-preview").length > before, 10000);
    if (!ok) throw new Error(t("missingImage"));
    await sleep(1200); // let the upload finish
  }

  function consentDialogOpen() {
    return $$("mat-dialog-container, .cdk-overlay-pane").some((d) => d.offsetParent !== null && /同意|동의|agree/i.test(d.innerText) && /上傳|上传|アップロード|업로드|upload/i.test(d.innerText));
  }
  function closeOverlays() {
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", code: "Escape", keyCode: 27, bubbles: true }));
    $(".cdk-overlay-backdrop")?.click();
  }

  async function pressSend(stillCurrent = () => true) {
    const btn = await waitFor(() => {
      if (!stillCurrent()) throw new Error(t("cancelSend"));
      const b = sendButton();
      return b && !b.disabled && b.getAttribute("aria-disabled") !== "true" ? b : null;
    }, 30000, 250);
    if (!btn) throw new Error(t("sendTimeout"));
    if (!stillCurrent()) throw new Error(t("cancelSend"));
    btn.click();
  }

  // ------------------------------------------------------------ parsing
  // Gemini sometimes puts raw newlines/tabs inside JSON strings; escape them.
  function repairJson(s) {
    let out = "", inStr = false, esc = false;
    for (const ch of s) {
      if (inStr) {
        if (esc) { out += ch; esc = false; continue; }
        if (ch === "\\") { out += ch; esc = true; continue; }
        if (ch === '"') { inStr = false; out += ch; continue; }
        if (ch === "\n") { out += "\\n"; continue; }
        if (ch === "\r") { continue; }
        if (ch === "\t") { out += "\\t"; continue; }
        out += ch;
      } else {
        if (ch === '"') inStr = true;
        out += ch;
      }
    }
    return out.replace(/,\s*([}\]])/g, "$1");
  }
  function tryParse(s) {
    for (const t of [s, repairJson(s)]) {
      try { return JSON.parse(t); } catch (_) { /* next */ }
    }
    return undefined;
  }
  // Pull every balanced {...} that contains "gf_tool" out of a text.
  function scanObjects(text) {
    const found = [];
    let idx = text.indexOf('"gf_tool"');
    while (idx !== -1) {
      let start = text.lastIndexOf("{", idx);
      if (start === -1) break;
      let depth = 0, inStr = false, esc = false, end = -1;
      for (let i = start; i < text.length; i++) {
        const ch = text[i];
        if (inStr) {
          if (esc) esc = false;
          else if (ch === "\\") esc = true;
          else if (ch === '"') inStr = false;
        } else if (ch === '"') inStr = true;
        else if (ch === "{") depth++;
        else if (ch === "}" && --depth === 0) { end = i; break; }
      }
      if (end === -1) break;
      const obj = tryParse(text.slice(start, end + 1));
      if (obj) found.push(obj);
      idx = text.indexOf('"gf_tool"', end);
    }
    return found;
  }
  function normalize(obj) {
    const list = Array.isArray(obj) ? obj : [obj];
    return list.filter((o) => o && typeof o.gf_tool === "string").map((o) => {
      const args = o.args && typeof o.args === "object" ? o.args : Object.fromEntries(Object.entries(o).filter(([k]) => k !== "gf_tool"));
      return { tool: o.gf_tool.trim(), args };
    });
  }
  function extractCalls(resp) {
    const body = $("message-content", resp) || resp;
    const calls = [];
    for (const code of $$("code", body)) {
      const t = code.innerText;
      if (!t.includes("gf_tool")) continue;
      const whole = tryParse(t.trim());
      if (whole !== undefined) calls.push(...normalize(whole));
      else scanObjects(t).forEach((o) => calls.push(...normalize(o)));
    }
    if (!calls.length) scanObjects(body.innerText).forEach((o) => calls.push(...normalize(o)));
    return calls; // repeats are kept on purpose (e.g. pressing the same key twice)
  }

  // ------------------------------------------------------------ main loop
  function markAllSeen() { responses().forEach((r) => seen.add(r)); }

  function checkUrl() {
    if (location.href === lastUrl) return;
    const oldPath = new URL(lastUrl).pathname.replace(/\/$/, "");
    const newPath = location.pathname;
    lastUrl = location.href;
    // A brand-new chat getting its id (/app -> /app/abc) is the same conversation.
    if (!newPath.startsWith(oldPath + "/")) {
      ignoreUntil = Date.now() + 4000;
      state.steps = 0;
      allowed = new Set();
    }
  }

  async function tick() {
    checkUrl();
    if (Date.now() < ignoreUntil) { markAllSeen(); return; }
    if (!state.enabled || state.busy) return;
    const last = responses().at(-1);
    if (!last || seen.has(last)) return;

    const text = (last.innerText || "").trim();
    const rec = track.get(last) || { text: "", stable: 0 };
    if (isGenerating() || !text) { rec.stable = 0; rec.text = text; track.set(last, rec); setStatus("generating"); return; }
    if (text === rec.text) rec.stable++; else { rec.stable = 0; rec.text = text; }
    track.set(last, rec);
    if (rec.stable < STABLE_TICKS) return;

    seen.add(last);
    // Same text can legitimately repeat (e.g. two screenshots in a row), so the
    // reply's position is part of the signature.
    const sig = hash(location.pathname + "|" + responses().indexOf(last) + "|" + text);
    if (handled.has(sig)) return;
    handled.add(sig);

    const calls = extractCalls(last);
    if (!calls.length) {
      if (state.steps) setStatus("completed", { n: state.steps });
      else setStatus("idle");
      state.steps = 0;
      return;
    }
    state.busy = true;
    try {
      await runCalls(calls);
    } catch (e) {
      setStatus("error", { detail: e.message }, true);
      log("❌ " + e.message);
    } finally {
      state.busy = false;
      state.abort = false;
      render();
    }
  }

  async function runCalls(calls) {
    state.abort = false;
    if (state.steps >= MAX_AUTO_STEPS) {
      const go = await ask("continueRounds", { n: state.steps }, null);
      if (go !== "run" && go !== "always") { setStatus("stopped"); return; }
      state.steps = 0;
    }
    const results = [];
    for (const [i, call] of calls.entries()) {
      if (state.abort) { setStatus("stopped"); return; }
      const needOk = !allowed.has(call.tool) &&
        (state.mode === "manual" || (state.mode === "readonly" && !READ_ONLY.has(call.tool)));
      if (needOk) {
        setStatus("waiting", { tool: call.tool });
        const ans = await ask("confirmTool", { i: i + 1, n: calls.length, tool: call.tool }, call);
        if (ans === "stop" || state.abort) { setStatus("stopped"); log("⏹ " + t("userStop")); return; }
        if (ans === "always") { allowed.add(call.tool); log("🔓 " + t("allowed", { tool: call.tool })); }
        if (ans === "skip") {
          results.push({ call, ok: false, output: t("denied") });
          log("⏭ " + t("skipped", { tool: call.tool }));
          continue;
        }
      }
      setStatus("running", { tool: call.tool });
      const r = await bg({ type: "tool", tool: call.tool, args: call.args });
      if (r.offline) throw new Error(r.error);
      results.push({ call, ok: !!r.ok, output: r.ok ? r.output : t("error", { detail: r.error }), image: r.image });
      log(`${r.ok ? "✅" : "⚠️"} ${call.tool} ${short(call.args)}`);
    }
    if (state.abort) { setStatus("stopped"); return; }
    await reply(results);
    state.steps++;
    setStatus("returned", { n: state.steps });
  }

  async function reply(results) {
    let text = t("result") + "\n";
    let shotNote = "";
    for (const [i, r] of results.entries()) {
      text += `\n--- ${i + 1}. ${r.call.tool} ${r.ok ? "✅" : "❌"} ---\n${r.output}\n`;
    }
    // Only the newest screenshot matters; uploading every one would be slow.
    const withImage = results.filter((x) => x.image);
    for (const r of withImage.slice(-1)) {
      setStatus("uploading");
      try {
        await uploadImage(r.image);
        shotNote = withImage.length > 1
          ? "\n" + t("lastShot", { tool: r.call.tool })
          : "\n" + t("latestShot");
      } catch (e) {
        shotNote = "\n" + t("failedShot", { detail: e.message });
        log("⚠️ " + e.message);
        if (e instanceof UploadConsentError) throw e;
      }
    }
    text += shotNote + "\n\n" + t("next");
    insertText(text);
    await sleep(400);
    await pressSend();
  }

  const short = (a) => { const s = JSON.stringify(a); return s.length > 80 ? s.slice(0, 80) + "…" : s; };

  // ------------------------------------------------------------ auto prompt
  // In a brand-new chat, hold the first send, put the tool instructions in
  // front of the user's text, then send it for real.
  async function loadPrompt() {
    await settingsReady;
    const selected = language;
    const r = await bg({ type: "prompt", language: selected });
    if (selected !== language) return loadPrompt();
    return r.ok ? r.text.trim() : "";
  }

  let bypassSend = false;
  // Gemini's native Tasks starter shares /app and the chat editor.
  const taskStarter = (ed) => /^(說明要執行的任務|说明要执行的任务|(?:実行する)?タスクを(?:説明|入力)|(?:수행할 )?작업을 (?:설명|입력)|describe (?:a |the |your )?task\b)/i.test(
    ed?.getAttribute("data-placeholder")?.trim() || "");
  function shouldInject() {
    if (bypassSend || !state.enabled || !state.autoPrompt) return false;
    if (location.pathname.includes("/gem/")) return false; // a Gem carries its own instructions
    if ($$("user-query").length || responses().length) return false; // not a new chat
    const ed = editor();
    if (taskStarter(ed)) return false;
    const text = ed ? ed.innerText : "";
    // A message that @-mentions the GeminiForge MCP app already has real tools.
    if (/@GeminiForge\b/i.test(text)) return false;
    return text.trim().length > 0 && !hasPrompt(text);
  }
  async function injectAndSend() {
    const originalEditor = editor();
    const originalUrl = location.href;
    const sameChat = () => editor() === originalEditor && location.href === originalUrl && !taskStarter(originalEditor);
    try {
      const p = await loadPrompt();
      if (!sameChat()) return;
      if (!p) throw new Error(t("noPrompt"));
      insertText(p + "\n\n" + t("request"), true);
      state.steps = 0;
      await sleep(350);
      if (!sameChat()) return;
      bypassSend = true;
      await pressSend(sameChat);
      log("📎 " + t("injected"));
    } catch (e) {
      setStatus("injectFailed", { detail: e.message }, true);
    } finally {
      bypassSend = false;
    }
  }
  function holdSend(e) {
    e.preventDefault();
    e.stopImmediatePropagation();
    injectAndSend();
  }
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Enter" || e.shiftKey || e.isComposing || e.keyCode === 229) return;
    if (!e.target.closest || !e.target.closest("rich-textarea")) return;
    if (shouldInject()) holdSend(e);
  }, true);
  document.addEventListener("click", (e) => {
    const btn = e.target.closest && e.target.closest("button");
    if (!btn || btn !== sendButton()) return;
    if (shouldInject()) holdSend(e);
  }, true);

  // ------------------------------------------------------------ panel UI
  const host = document.createElement("div");
  host.style.cssText = "position:fixed;right:16px;bottom:96px;z-index:2147483647;";
  const root = host.attachShadow({ mode: "open" });
  root.innerHTML = `
  <style>
    :host{all:initial}
    .p{width:340px;font:13px/1.45 "Segoe UI",system-ui,sans-serif;color:#e3e3e3;background:#1e1f20;
       border:1px solid #3c4043;border-radius:14px;box-shadow:0 6px 24px rgba(0,0,0,.45);overflow:hidden}
    .h{display:flex;align-items:center;gap:8px;padding:9px 12px;background:#282a2c;cursor:pointer;user-select:none}
    .h b{flex:1;font-size:14px}
    .dot{width:9px;height:9px;border-radius:50%;background:#888}
    .dot.on{background:#34a853}.dot.off{background:#ea4335}
    .b{padding:10px 12px;display:flex;flex-direction:column;gap:8px}
    .p.min .b{display:none}
    .row{display:flex;gap:6px;align-items:center;flex-wrap:wrap}
    select,button{font:inherit;color:#e3e3e3;background:#303134;border:1px solid #5f6368;border-radius:8px;padding:5px 9px;cursor:pointer}
    button:hover{background:#3c4043}
    button.pri{background:#1a73e8;border-color:#1a73e8;color:#fff}
    button.warn{background:#5c2b29;border-color:#8c3b36}
    label{display:flex;align-items:center;gap:5px;cursor:pointer}
    .st{color:#9aa0a6;min-height:18px;word-break:break-all}
    .st.err{color:#f28b82}
    .ask{border:1px solid #fbbc04;border-radius:10px;padding:8px;background:#2b2a1f;display:none}
    .ask.show{display:block}
    .ask pre{margin:6px 0;max-height:180px;overflow:auto;white-space:pre-wrap;word-break:break-all;font:12px Consolas,monospace;
             background:#151515;padding:6px;border-radius:6px}
    .log{max-height:130px;overflow:auto;font:12px Consolas,monospace;color:#bdc1c6;background:#151515;border-radius:8px;padding:6px}
    .log div{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  </style>
  <div class="p">
    <div class="h" id="hd"><span class="dot" id="dot"></span><b>⚒ GeminiForge</b><span id="tg">－</span></div>
    <div class="b">
      <label><span data-msg="language"></span><select id="language"></select></label>
      <div class="row">
        <label><input type="checkbox" id="en"><span data-msg="enabled"></span></label>
        <select id="mode"></select>
      </div>
      <div class="st" data-msg="modeNote"></div>
      <label data-tip="autoHint"><input type="checkbox" id="ap"><span data-msg="autoPrompt"></span></label>
      <div class="row">
        <button id="prompt" class="pri" data-msg="prompt"></button>
        <button id="stop" class="warn" data-msg="stop"></button>
        <button id="shot" data-msg="shot"></button>
        <button id="reload" data-tip="reload">↻</button>
      </div>
      <div class="st" id="st"></div>
      <div class="ask" id="ask">
        <div id="askt"></div>
        <pre id="askc"></pre>
        <div class="row">
          <button class="pri" id="run" data-msg="run"></button>
          <button id="always" data-tip="alwaysHint" data-msg="always"></button>
          <button id="skip" data-msg="skip"></button>
          <button class="warn" id="halt" data-msg="halt"></button>
        </div>
      </div>
      <div class="log" id="log"></div>
    </div>
  </div>`;
  const el = (id) => root.getElementById(id);
  for (const [code, name] of Object.entries(GF_LANGUAGES)) {
    el("language").add(new Option(name, code));
  }
  for (const mode of MODES) {
    const option = new Option("", mode);
    option.dataset.msg = mode;
    el("mode").add(option);
  }

  let status = { key: "idle", values: {} };
  let question;
  let connection;
  function render() {
    host.lang = language;
    for (const node of $$("[data-msg]", root)) node.textContent = t(node.dataset.msg);
    for (const node of $$("[data-tip]", root)) node.title = t(node.dataset.tip);
    el("mode").setAttribute("aria-label", t("mode"));
    el("reload").setAttribute("aria-label", t("reload"));
    el("language").value = language;
    el("en").checked = state.enabled;
    el("ap").checked = state.autoPrompt;
    el("mode").value = state.mode;
    el("st").textContent = t(status.key, status.values);
    if (question) el("askt").textContent = t(question.key, question.values);
    renderConnection();
  }
  function setStatus(key, values = {}, err = false) {
    status = { key, values };
    el("st").textContent = t(key, values);
    el("st").classList.toggle("err", err);
  }
  function log(t) {
    const d = document.createElement("div");
    d.textContent = `${new Date().toLocaleTimeString(language)} ${t}`;
    d.title = t;
    el("log").prepend(d);
    while (el("log").children.length > 40) el("log").lastChild.remove();
  }

  let askResolve = null;
  function ask(key, values, call) {
    question = { key, values };
    el("askt").textContent = t(key, values);
    el("askc").textContent = call ? JSON.stringify(call.args, null, 2).slice(0, 3000) : "";
    el("askc").style.display = call ? "block" : "none";
    el("ask").classList.add("show");
    root.querySelector(".p").classList.remove("min");
    return new Promise((r) => { askResolve = r; });
  }
  function answer(v) {
    question = null;
    el("ask").classList.remove("show");
    const r = askResolve; askResolve = null;
    r && r(v);
  }

  el("language").onchange = (e) => chrome.storage.local.set({ language: e.target.value });
  el("hd").onclick = () => {
    const p = root.querySelector(".p");
    p.classList.toggle("min");
    el("tg").textContent = p.classList.contains("min") ? "＋" : "－";
  };
  el("en").onchange = (e) => { state.enabled = e.target.checked; save(); setStatus(state.enabled ? "idle" : "disabled"); };
  el("mode").onchange = (e) => { state.mode = e.target.value; save(); };
  el("ap").onchange = (e) => { state.autoPrompt = e.target.checked; save(); };
  el("reload").onclick = async () => {
    if (state.busy) return setStatus("busy", {}, true);
    await bg({ type: "reload" });
    setTimeout(() => location.reload(), 800);
  };
  el("run").onclick = () => answer("run");
  el("always").onclick = () => answer("always");
  el("skip").onclick = () => answer("skip");
  el("halt").onclick = () => { state.abort = true; answer("stop"); };
  el("stop").onclick = () => { state.abort = true; state.steps = 0; answer("stop"); setStatus("stopped"); };
  el("prompt").onclick = async () => {
    const p = await loadPrompt();
    if (!p) return setStatus("noPrompt", {}, true);
    const ed = editor();
    if (ed && hasPrompt(ed.innerText)) return setStatus("hasPrompt");
    const empty = !ed || !ed.innerText.trim();
    insertText(p + (empty ? "\n\n" + t("request") : "\n\n"), true);
    state.steps = 0;
    setStatus("inserted");
  };
  el("shot").onclick = async () => {
    setStatus("testing");
    const r = await bg({ type: "tool", tool: "screenshot", args: {} });
    if (!r.ok) return setStatus("error", { detail: r.error }, true);
    try { await uploadImage(r.image); setStatus("attached"); }
    catch (e) { setStatus("error", { detail: e.message }, true); }
  };

  async function health() {
    connection = await bg({ type: "health" });
    renderConnection();
  }
  function renderConnection() {
    if (!connection) return;
    el("dot").className = "dot " + (connection.ok ? "on" : "off");
    el("dot").title = connection.ok ? t("connected", { cwd: connection.cwd }) : t("offline");
  }

  chrome.storage.onChanged.addListener((changes, area) => {
    if (area === "local" && changes.language) {
      applyLanguage(geminiForgeLanguage(changes.language.newValue || chrome.i18n.getUILanguage()));
      health();
    }
  });
  const settingsReady = chrome.storage.local.get(["enabled", "mode", "autoPrompt", "language"]).then(async v => {
    if (typeof v.enabled === "boolean") state.enabled = v.enabled;
    if (typeof v.autoPrompt === "boolean") state.autoPrompt = v.autoPrompt;
    if (MODES.includes(v.mode)) state.mode = v.mode;
    if (!state.enabled) setStatus("disabled");
    await applyLanguage(geminiForgeLanguage(v.language || chrome.i18n.getUILanguage()));
    document.documentElement.appendChild(host);
  });
  health();
  setInterval(health, 5000);
  setInterval(() => tick().catch((e) => console.error("[GeminiForge]", e)), 1000);
})();
