// GeminiForge content script for gemini.google.com
// Watches Gemini's replies for {"gf_tool": ...} blocks, runs them through the
// local server (with the user's approval), and sends the results back.
(() => {
  if (window.__geminiForge) return;
  window.__geminiForge = true;

  // ------------------------------------------------------------ settings
  const READ_ONLY = new Set(["cd", "list_dir", "read_file", "read_files", "search", "find_files", "screenshot", "wait",
    "wait_for", "job_status", "list_backups", "clipboard_get", "list_windows", "ui_elements", "read_window",
    "ocr_window", "zoom"]);
  const MAX_AUTO_STEPS = 200;     // ask before continuing after this many rounds
  const STABLE_TICKS = 2;         // reply must stop changing for this many seconds
  const hasPrompt = (text) => /GeminiForge\s*(?:tool mode|tools\]|ツールモード|도구 모드|工具模式)/i.test(text);
  const MODES = ["manual", "readonly", "auto"];
  let language = geminiForgeLanguage(chrome.i18n.getUILanguage());
  let messages = {};
  function t(key, values = {}) {
    let template = messages[key]?.message;
    if (!template) {
      try { template = chrome.i18n?.getMessage?.(key); } catch (_) { template = ""; }
    }
    return (template || key).replace(/\{(\w+)\}/g, (match, name) => values[name] ?? match);
  }

  // When to show desktop notifications, mapped to the label of each choice.
  const NOTIFY = { off: "notifyOff", away: "notifyAway", always: "notifyAlways" };
  const state = { enabled: true, mode: "readonly", autoPrompt: true, notify: "away", busy: false, steps: 0, abort: false };
  let allowed = new Set();         // tools the user allowed for the current chat
  const seen = new WeakSet();
  const handled = new Set();
  const track = new WeakMap();
  let ignoreUntil = Date.now() + 4000; // existing chat history is never executed
  // History can also arrive, or be drawn again, long after that. So a reply is
  // acted on only while a run is live: this tab sent the message it answers,
  // or saw Gemini writing it.
  let live = false;
  let lastUrl = location.href;
  async function applyLanguage(selected) {
    language = selected;
    const response = await bg({ type: "messages", language: selected });
    if (language !== selected) return;
    messages = response.messages || {};
    render();
  }

  const save = () => chrome.storage.local.set({ enabled: state.enabled, mode: state.mode, autoPrompt: state.autoPrompt, notify: state.notify });

  // ------------------------------------------------------------ utils
  // Chrome slows the timers of a hidden tab (another tab in front, or the window
  // minimised or covered) to one a second, then one a minute. The service
  // worker's timers keep their pace, so a hidden tab sleeps there instead.
  const localSleep = (ms) => new Promise((r) => setTimeout(r, ms));
  async function sleep(ms) {
    if (!document.hidden) return localSleep(ms);
    const end = Date.now() + ms;
    await Promise.race([localSleep(ms), bg({ type: "sleep", ms })]);
    if (Date.now() < end - 20) await localSleep(end - Date.now()); // the worker was not available
  }
  const $ =(s, root = document) => root.querySelector(s);
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
  // A desktop notification, either always or only while the user is looking
  // at another tab, window or app; coming back to this tab clears it.
  const away = () => document.visibilityState !== "visible" || !document.hasFocus();
  const clip = (s, n = 160) => {
    s = String(s || "").replace(/\s+/g, " ").trim();
    return s.length > n ? s.slice(0, n) + "…" : s;
  };
  async function sendNotice(title, message = "") {
    const r = await bg({ type: "notify", title, message: clip(message) });
    if (!r.ok) log("⚠️ " + t("notifyFailed", { detail: r.error }));
    return r;
  }
  function notify(title, message = "") {
    if (state.notify === "off" || (state.notify === "away" && !away())) return;
    log("🔔 " + title);
    sendNotice(title, message);
  }
  const clearNotify = () => { if (!away()) bg({ type: "clearNotify" }); };
  window.addEventListener("focus", clearNotify);
  document.addEventListener("visibilitychange", clearNotify);

  async function waitFor(fn, timeout = 5000, every = 150) {
    const end = Date.now() + timeout;
    for (;;) {
      const v = fn();
      if (v) return v;
      if (Date.now() >= end) return null; // checked once more after a sleep that ran late
      await sleep(every);
    }
  }

  // ------------------------------------------------------------ Gemini page
  const editor = () => $("rich-textarea .ql-editor") || $('div[contenteditable="true"][role="textbox"]');
  const responses = () => $$("model-response");
  const stopButton = () =>
    $$("button").find((b) => /停止|中止|중지|중단|stop/i.test(b.getAttribute("aria-label") || "") && b.offsetParent !== null);
  const isGenerating = () => !!stopButton();

  // gemini.google.com/usage lists each limit as an <li> holding a bold title,
  // "N% used" and a reset time. Its class names are generated, so only that
  // structure and the "%" are relied on; labels stay in Gemini's UI language.
  const isUsagePage = () => /\/usage\/?$/.test(location.pathname);
  function parseUsage(doc) {
    const rows = [];
    for (const li of doc.querySelectorAll("main section ul > li")) {
      const leaves = [...li.querySelectorAll("*")]
        .filter((e) => !e.children.length && e.textContent.trim())
        .map((e) => ({ tag: e.tagName, text: e.textContent.trim().replace(/\s+/g, " ") }));
      const pct = leaves.find((l) => /\d\s*%|%\s*\d/.test(l.text));
      if (!pct) continue;
      const others = leaves.filter((l) => l !== pct);
      const label = (others.find((l) => l.tag === "STRONG" && !/\d/.test(l.text)) || others.find((l) => !/\d/.test(l.text)))?.text || "";
      const reset = others.find((l) => /\d/.test(l.text))?.text || "";
      rows.push({ label: label.slice(0, 60), percent: Number(pct.text.match(/\d+(?:\.\d+)?/)[0]), reset: reset.slice(0, 80) });
    }
    return rows;
  }
  let reporting = false;
  async function reportUsage() {
    if (reporting) return;
    reporting = true;
    try {
      const rows = await waitFor(() => { const r = parseUsage(document); return r.length ? r : null; }, 20000, 300);
      if (isUsagePage()) await bg({ type: "usageData", account: accountIndex(), rows: rows || [], error: rows ? "" : "notFound" });
    } finally {
      reporting = false;
    }
  }

  function sendButton() {
    return $("button.send-button") || $('button[aria-label*="Send message"]') ||
      $$("input-area-v2 button, input-container button").find((b) => /send|傳送|发送|送信|보내기|전송/i.test(b.getAttribute("aria-label") || ""));
  }
  function uploadMenuButton() {
    return $$("input-area-v2 button, input-container button").find((b) => /upload|add file|上傳|上传|アップロード|업로드/i.test(b.getAttribute("aria-label") || ""));
  }

  function caretAt(ed, atStart) {
    ed.focus();
    const sel = window.getSelection();
    const range = document.createRange();
    range.selectNodeContents(ed);
    range.collapse(atStart);
    sel.removeAllRanges();
    sel.addRange(range);
  }
  // execCommand("insertText") lays the page out again for every line it
  // inserts: a 300-line tool result froze the tab for four seconds. page.js,
  // in the page's own world, puts the text in through Quill in one step and
  // cancels this event once it is in. (A simulated paste is no substitute:
  // Gemini takes it later and turns "1. " and "- " lines into lists.)
  // Otherwise the text is typed in as before.
  function insertText(text, atStart = false) {
    const ed = editor();
    if (!ed) throw new Error(t("noEditor"));
    caretAt(ed, atStart);
    const request = new CustomEvent("geminiforge-insert", { detail: JSON.stringify({ text, atStart }), bubbles: true, cancelable: true });
    if (!ed.dispatchEvent(request)) return;
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
    live = true;
    btn.click();
  }

  // ------------------------------------------------------------ parsing
  // Gemini sometimes puts raw newlines/tabs inside JSON strings; escape them.
  function repairJson(s) {
    let out = "", inStr = false, esc = false;
    for (let i = 0; i < s.length; i++) {
      const ch = s[i];
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
        if (ch === ",") {
          let j = i + 1;
          while (j < s.length && /\s/.test(s[j])) j++;
          if (s[j] === "}" || s[j] === "]") continue;
        }
        out += ch;
      }
    }
    return out;
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
      const start = text.lastIndexOf("{", idx);
      if (start === -1) { idx = text.indexOf('"gf_tool"', idx + 9); continue; }
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
      if (end === -1) { idx = text.indexOf('"gf_tool"', idx + 9); continue; }
      const obj = tryParse(text.slice(start, end + 1));
      if (obj) found.push(obj);
      idx = text.indexOf('"gf_tool"', Math.max(end + 1, idx + 9));
    }
    return found;
  }
  function normalize(obj) {
    const list = Array.isArray(obj) ? obj : [obj];
    return list.filter((o) => o && typeof o.gf_tool === "string").map((o) => {
      const args = o.args && typeof o.args === "object" && !Array.isArray(o.args) ? o.args : Object.fromEntries(Object.entries(o).filter(([k]) => k !== "gf_tool"));
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

  function sameConversation(baseUrl) {
    if (location.href === baseUrl) return true;
    const oldPath = new URL(baseUrl).pathname.replace(/\/$/, "");
    return location.pathname.startsWith(oldPath + "/");
  }

  function checkUrl() {
    if (location.href === lastUrl) return;
    const prevUrl = lastUrl;
    lastUrl = location.href;
    if (isUsagePage()) reportUsage();
    // A brand-new chat getting its id (/app -> /app/abc) is the same conversation.
    if (!sameConversation(prevUrl)) {
      ignoreUntil = Date.now() + 4000;
      live = false;
      state.steps = 0;
      allowed = new Set();
      stall = { sig: "", since: 0, retries: 0, gaveUp: false };
      if (askResolve) {
        state.abort = true;
        answer("stop");
      }
    }
  }

  // Gemini sometimes hangs on "responding" and never writes anything. During a
  // tool run, a reply that shows no change for this long is stopped and asked
  // for again; after a few tries in a row the user is told instead.
  const STALL_MS = 120000;
  const MAX_STALL_RETRIES = 2;
  let stall = { sig: "", since: 0, retries: 0, gaveUp: false };
  function stalled(sig, generating, now) {
    if (!generating || stall.gaveUp) { stall.since = 0; return false; }
    if (!stall.since || sig !== stall.sig) { stall.sig = sig; stall.since = now; return false; }
    return now - stall.since >= STALL_MS;
  }
  async function recoverStall() {
    const chatUrl = location.href;
    state.abort = false;
    const stillCurrent = () => !state.abort && sameConversation(chatUrl);
    stall.since = 0;
    if (stall.retries >= MAX_STALL_RETRIES) {
      stall.gaveUp = true;
      throw new Error(t("stallGiveUp"));
    }
    stall.retries++;
    setStatus("stalled", { s: STALL_MS / 1000 });
    log("⚠️ " + t("stalled", { s: STALL_MS / 1000 }));
    stopButton()?.click();
    if (!(await waitFor(() => !isGenerating(), 10000, 250))) throw new Error(t("stallGiveUp"));
    await sleep(1000); // Gemini may put the stopped message back into the editor
    markAllSeen();     // whatever the stopped reply left behind is not a reply to act on
    if (!stillCurrent()) throw new Error(t("cancelSend"));
    const restored = (editor()?.innerText || "").trim().startsWith(t("result"));
    if (!restored) insertText(t("stallNudge"));
    await sleep(400);
    await pressSend(stillCurrent);
  }
  // One piece of work at a time; a failure is shown, logged and notified.
  async function exclusive(work) {
    state.busy = true;
    try {
      await work();
    } catch (e) {
      setStatus("error", { detail: e.message }, true);
      log("❌ " + e.message);
      notify(t("error", { detail: e.message }));
    } finally {
      state.busy = false;
      state.abort = false;
      render();
    }
  }

  async function tick() {
    checkUrl();
    if (Date.now() < ignoreUntil) { markAllSeen(); return; }
    if (isGenerating()) live = true;
    if (!state.enabled || state.busy) return;
    const last = responses().at(-1);
    const progress = responses().length + "|" + (last?.innerText || "").length;
    if (stalled(progress, state.steps > 0 && isGenerating(), Date.now())) return exclusive(recoverStall);
    if (!last || seen.has(last)) return;

    const text = (last.innerText || "").trim();
    const rec = track.get(last) || { text: "", stable: 0 };
    if (isGenerating() || !text) { rec.stable = 0; rec.text = text; track.set(last, rec); setStatus("generating"); return; }
    if (text === rec.text) rec.stable++; else { rec.stable = 0; rec.text = text; }
    track.set(last, rec);
    if (rec.stable < STABLE_TICKS) return;

    seen.add(last);
    if (!live) return;   // it was already in the chat
    stall.retries = 0;
    stall.gaveUp = false;
    // Same text can legitimately repeat (e.g. two screenshots in a row), so the
    // reply's position is part of the signature.
    const sig = hash(location.pathname + "|" + responses().indexOf(last) + "|" + text);
    if (handled.has(sig)) return;
    handled.add(sig);

    const calls = extractCalls(last);
    if (!calls.length) {
      refreshUsage(false); // a finished reply used some of the limit
      notify(state.steps ? t("completed", { n: state.steps }) : t("notifyReply"), ($("message-content", last) || last).innerText);
      if (state.steps) setStatus("completed", { n: state.steps });
      else setStatus("idle");
      state.steps = 0;
      live = false;
      return;
    }
    await exclusive(() => runCalls(calls));
  }

  async function runCalls(calls) {
    const chatUrl = location.href;
    state.abort = false;
    if (state.steps >= MAX_AUTO_STEPS) {
      const go = await ask("continueRounds", { n: state.steps }, null);
      if ((go !== "run" && go !== "always") || !sameConversation(chatUrl)) { setStatus("stopped"); return; }
      state.steps = 0;
    }
    const results = [];
    for (const [i, call] of calls.entries()) {
      if (state.abort || !sameConversation(chatUrl)) { setStatus("stopped"); return; }
      const needOk = !allowed.has(call.tool) &&
        (state.mode === "manual" || (state.mode === "readonly" && !READ_ONLY.has(call.tool)));
      if (needOk) {
        setStatus("waiting", { tool: call.tool });
        const ans = await ask("confirmTool", { i: i + 1, n: calls.length, tool: call.tool }, call);
        if (ans === "stop" || state.abort || !sameConversation(chatUrl)) { setStatus("stopped"); log("⏹ " + t("userStop")); return; }
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
    if (state.abort || !sameConversation(chatUrl)) { setStatus("stopped"); return; }
    await reply(results, chatUrl);
    state.steps++;
    setStatus("returned", { n: state.steps });
  }

  async function reply(results, chatUrl = location.href) {
    const stillCurrent = () => !state.abort && sameConversation(chatUrl);
    let text = t("result") + "\n";
    let shotNote = "";
    for (const [i, r] of results.entries()) {
      text += `\n--- ${i + 1}. ${r.call.tool} ${r.ok ? "✅" : "❌"} ---\n${r.output}\n`;
    }
    // Only the newest screenshot matters; uploading every one would be slow.
    const withImage = results.filter((x) => x.image);
    for (const r of withImage.slice(-1)) {
      if (!stillCurrent()) throw new Error(t("cancelSend"));
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
    if (!stillCurrent()) throw new Error(t("cancelSend"));
    text += shotNote + "\n\n" + t("next");
    insertText(text);
    await sleep(400);
    if (!stillCurrent()) throw new Error(t("cancelSend"));
    await pressSend(stillCurrent);
  }

  const short = (a) => { const s = JSON.stringify(a); return s.length > 80 ? s.slice(0, 80) + "…" : s; };

  // ------------------------------------------------------------ auto prompt
  // In a brand-new chat, hold the first send, put the tool instructions in
  // front of the user's text, then send it for real.
  async function loadPrompt() {
    await settingsReady;
    const selected = language;
    // Servers on macOS and Linux report their os; Windows servers do not.
    const r = await bg({ type: "prompt", language: selected, os: connection?.os });
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
    else if (editor()?.innerText.trim()) live = true;
  }, true);
  document.addEventListener("click", (e) => {
    const btn = e.target.closest && e.target.closest("button");
    if (!btn || btn !== sendButton()) return;
    if (shouldInject()) holdSend(e);
    else live = true;
  }, true);

  // ------------------------------------------------------------ handoff
  // A chat belongs to one Google account. When that account runs out of usage,
  // the task is carried to another signed-in account (gemini.google.com/u/N):
  // this tab packs the request and the progress so far, the service worker
  // opens a new chat there, and the content script in that tab sends the pack.
  const HANDOFF_TASK = 4000;      // characters kept of the original request
  const HANDOFF_BUDGET = 14000;   // characters of history carried over
  const HANDOFF_RECENT = 6;       // newest messages kept (almost) whole
  const HANDOFF_TYPED = 6000;     // more characters, for older messages the user typed
  const REQUEST_MARK = /(?:My request:|依頼内容：|요청 사항:|我的需求：)\s*/;
  const HANDOFF_MARKS = ["=== TASK ===", "=== LOG ==="]; // the same in every language, so a pack can be read back
  const accountIndex = (pathname = location.pathname) => Number(pathname.match(/^\/u\/(\d+)(?:\/|$)/)?.[1] || 0);

  // The accounts signed in to this browser, by their /u/N number. Google has
  // no public list, so each /u/N/app page is fetched (same origin, nothing
  // leaves the browser) and the address is read out of its page data.
  const ACCOUNTS_MAX = 10;
  const ACCOUNTS_AGE = 30 * 60 * 1000;      // looked up again after this long
  const USAGE_FRESH = 3 * 60 * 60 * 1000;   // older usage says little about now
  let accounts = [];
  let handoffTo = null;                     // the account the user last picked
  function pageAccount(html) {
    const email = (html.match(/"oPEP7c":"([^"\s]+@[^"\s]+)"/) || html.match(/"([\w.+-]+@[\w-]+(?:\.[\w-]+)+)"/))?.[1] || "";
    const index = html.match(/"QrtxK":"(\d+)"/)?.[1];
    return { email, index: index === undefined ? null : Number(index) };
  }
  async function findAccounts() {
    const found = await Promise.all(Array.from({ length: ACCOUNTS_MAX }, async (_, i) => {
      try {
        const r = await fetch(`/u/${i}/app`, { credentials: "include" });
        if (!r.ok) return null;
        const a = pageAccount(await r.text());
        // A number nobody is signed in to is answered with another account's page.
        const other = a.index !== null ? a.index !== i : i > 0 && accountIndex(new URL(r.url).pathname) !== i;
        return a.email && !other ? { index: i, email: a.email } : null;
      } catch (_) {
        return null; // sent to the sign-in page
      }
    }));
    const list = found.filter(Boolean).filter((a, i, all) => all.findIndex((b) => b.email === a.email) === i);
    if (list.length) await chrome.storage.local.set({ accounts: { list, at: Date.now() } });
  }
  const accountName = (index) => accounts.find((a) => a.index === index)?.email || t("handoffAccount", { n: index + 1 });
  const usedPercent = (index) => {
    const u = usageAll[index];
    return u?.rows?.length && Date.now() - u.at < USAGE_FRESH ? Math.max(...u.rows.map((r) => r.percent)) : null;
  };
  // Every other account, with its usage when known. The user's pick stays
  // selected unless that account is known to be used up.
  function renderAccounts() {
    const box = el("acct");
    const list = accounts.length ? accounts : Array.from({ length: 6 }, (_, index) => ({ index }));
    box.replaceChildren();
    for (const a of list) {
      if (a.index === accountIndex()) continue;
      const pct = usedPercent(a.index);
      box.add(new Option(accountName(a.index) + (pct === null ? "" : ` · ${pct}%`), a.index));
    }
    const free = [...box.options].filter((o) => !(usedPercent(Number(o.value)) >= 100));
    const pick = free.find((o) => Number(o.value) === handoffTo) || free[0] || [...box.options].find((o) => Number(o.value) === handoffTo);
    if (pick) box.value = pick.value;
  }

  // The first message of a tool chat is the tool instructions and then the
  // request, or an earlier handoff when the task has already changed accounts.
  function stripPrompt(text, prompt = "") {
    const from = text.indexOf(HANDOFF_MARKS[0]);
    const to = text.indexOf(HANDOFF_MARKS[1], from);
    if (from !== -1 && to !== -1) return text.slice(from + HANDOFF_MARKS[0].length, to).trim();
    const mark = text.match(REQUEST_MARK);
    if (mark) return text.slice(mark.index + mark[0].length).trim();
    // Inserted by hand in front of a request: skip as many characters as the
    // instructions have, ignoring whitespace (the page reflows it).
    let left = prompt.replace(/\s/g, "").length;
    if (!left || !text.replace(/\s/g, "").startsWith(prompt.replace(/\s/g, ""))) return text.trim();
    let i = 0;
    for (; left; i++) if (!/\s/.test(text[i])) left--;
    return text.slice(i).trim();
  }
  // Newest messages matter most: they are kept nearly whole, older ones are cut
  // short, and whatever no longer fits the budget is left out and counted.
  // Newest first until the budget runs out. What the user typed themselves is
  // short and holds the instructions given along the way, so those messages
  // are kept from further back, out of an allowance of their own.
  function packHandoff(turns, budget = HANDOFF_BUDGET, spare = HANDOFF_TYPED) {
    const cut = (s, n) => (s.length > n ? s.slice(0, n) + ` …[+${s.length - n}]` : s);
    const kept = [];
    let skipped = 0;
    let full = false;
    for (let i = turns.length - 1; i >= 0; i--) {
      const typed = turns[i].role === "user" && !/^\[GeminiForge/.test(turns[i].text);
      const text = cut(turns[i].text, turns.length - i <= HANDOFF_RECENT ? 4000 : typed ? 1500 : 400);
      full ||= text.length > budget;
      if (full && !(typed && text.length <= spare)) { skipped++; continue; }
      if (full) spare -= text.length; else budget -= text.length;
      kept.unshift({ role: turns[i].role, text });
    }
    return { kept, skipped };
  }
  function queryText(node) {
    const lines = $$(".query-text-line", node);
    return (lines.length ? lines.map((l) => l.textContent).join("\n") : ($(".query-text", node) || node).innerText || "").trim();
  }
  function collectTurns() {
    const turns = $$("user-query, model-response").map((n) => n.matches("user-query")
      ? { role: "user", text: queryText(n) }
      : { role: "model", text: (($("message-content", n) || n).innerText || "").trim() }).filter((x) => x.text);
    // Results that could not be sent because the limit was hit are still in the editor.
    const draft = (editor()?.innerText || "").trim();
    if (/^\[GeminiForge/.test(draft)) turns.push({ role: "user", text: draft });
    return turns;
  }
  // A reopened long chat only holds its newest messages; scrolling to its top
  // makes Gemini fetch older ones, a batch at a time and seconds apart. Done
  // when the first message is the one that carries the tool instructions (or
  // an earlier handoff), or when nothing more has arrived for a while.
  const startsChat = (text) => hasPrompt(text) || text.includes(HANDOFF_MARKS[0]);
  async function loadHistory() {
    const turns = () => $$("user-query, model-response");
    const atStart = () => { const q = $("user-query"); return !!q && startsChat(queryText(q)); };
    let scroller = turns()[0]?.parentElement;
    while (scroller && !(scroller.scrollHeight > scroller.clientHeight + 10 && /auto|scroll/.test(getComputedStyle(scroller).overflowY))) {
      scroller = scroller.parentElement;
    }
    const scrollers = [...new Set([scroller, $("infinite-scroller")])].filter(Boolean);
    let mark = "";
    for (let i = 0, idle = 0; i < 360 && idle < 12 && turns().length && !atStart() && !state.abort; i++) {
      turns()[0].scrollIntoView({ block: "start" });
      for (const s of scrollers) {
        s.scrollTop = 0;
        s.dispatchEvent(new Event("scroll")); // already at the top: no event of its own
      }
      await sleep(500);
      const now = turns().length + "|" + scrollers.map((s) => s.scrollHeight).join("|");
      if (now !== mark) { mark = now; idle = 0; setStatus("handoffCollecting", { n: turns().length }); } else idle++;
    }
    turns().at(-1)?.scrollIntoView({ block: "end" });
  }
  async function startHandoff() {
    if (state.busy || isGenerating()) return setStatus("busy", {}, true);
    if (!el("acct").options.length) return setStatus("handoffNoAccount", {}, true);
    const to = Number(el("acct").value);
    await exclusive(async () => {
      setStatus("handoffCollecting", { n: $$("user-query, model-response").length });
      await loadHistory();
      if (state.abort) return setStatus("stopped");
      const turns = collectTurns();
      if (!turns.length) throw new Error(t("handoffEmpty"));
      // A chat whose first loaded message is one of ours did not load from its
      // start: the log still goes over, and Gemini is told the request is missing.
      const first = turns.findIndex((x) => x.role === "user");
      const whole = first !== -1 && !/^\[GeminiForge/.test(turns[first].text);
      const task = whole ? stripPrompt(turns[first].text, await loadPrompt()) : t("handoffNoTask");
      if (!whole) log("⚠️ " + t("handoffPartial"));
      const { kept, skipped } = packHandoff(whole ? turns.slice(first + 1) : turns);
      let text = t("handoffIntro") + "\n\n" + HANDOFF_MARKS[0] + "\n" +
        (task.length > HANDOFF_TASK ? task.slice(0, HANDOFF_TASK) + " …" : task) + "\n\n" + HANDOFF_MARKS[1] + "\n";
      if (skipped) text += t("handoffSkipped", { n: skipped }) + "\n";
      for (const x of kept) text += `\n${x.role === "user" ? "[User]" : "[Gemini]"}\n${x.text}\n`;
      text += "\n" + t("handoffNext");
      const r = await bg({ type: "handoff", to, text });
      if (!r.ok) throw new Error(r.error);
      setStatus("handoffOpened", { name: accountName(to) });
      log("➡️ " + t("handoffOpened", { name: accountName(to) }));
    });
  }
  // In the tab the service worker opened for a handoff: start the new chat.
  async function resumeHandoff() {
    if (isUsagePage() || !(await waitFor(editor, 30000, 300))) return;
    const { handoff } = await bg({ type: "handoffTake" });
    if (!handoff) return;
    await exclusive(async () => {
      // Google sends an account that is not signed in back to the default one.
      if (accountIndex() !== handoff.to) {
        findAccounts(); // the list offered an account that is no longer there
        throw new Error(t("handoffWrongAccount", { name: accountName(handoff.to) }));
      }
      setStatus("handoffResuming");
      const p = await loadPrompt();
      if (!p) throw new Error(t("noPrompt"));
      const chatUrl = location.href;
      const sameChat = () => sameConversation(chatUrl) && !$$("user-query").length;
      await sleep(Math.max(0, ignoreUntil - Date.now())); // replies before then are taken for old ones
      if (!sameChat()) throw new Error(t("cancelSend"));
      insertText(p + "\n\n" + handoff.text, true);
      state.steps = 0;
      await sleep(400);
      bypassSend = true;
      try { await pressSend(sameChat); } finally { bypassSend = false; }
      setStatus("handoffResumed");
      log("⬅️ " + t("handoffResumed"));
    });
  }

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
    #acct{flex:1;min-width:0}
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
    .us{display:flex;gap:8px;align-items:flex-start;border:1px solid #3c4043;border-radius:10px;padding:6px 8px}
    .us .ur{flex:1;display:flex;flex-direction:column;gap:2px;min-width:0}
    .us .ur div{display:flex;gap:6px;white-space:nowrap}
    .us .ur .rs{color:#9aa0a6;overflow:hidden;text-overflow:ellipsis}
    .us .pc{font-weight:600}
    .us .hot{color:#f28b82}
    .us button{padding:2px 8px}
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
        <label data-tip="notifyHint"><span data-msg="notify"></span><select id="nt"></select></label>
        <button id="ntest" data-msg="notifyTest"></button>
      </div>
      <div class="us" id="usage">
        <span data-msg="usage"></span>
        <div class="ur" id="usageRows"></div>
        <button id="usageRefresh" data-tip="usageRefresh">↻</button>
      </div>
      <div class="row">
        <button id="prompt" class="pri" data-msg="prompt"></button>
        <button id="stop" class="warn" data-msg="stop"></button>
        <button id="shot" data-msg="shot"></button>
        <button id="reload" data-tip="reload">↻</button>
      </div>
      <div class="row">
        <button id="handoff" data-tip="handoffHint" data-msg="handoff"></button>
        <select id="acct"></select>
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
  for (const [when, key] of Object.entries(NOTIFY)) {
    const option = new Option("", when);
    option.dataset.msg = key;
    el("nt").add(option);
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
    el("nt").value = state.notify;
    el("mode").value = state.mode;
    el("acct").setAttribute("aria-label", t("handoffHint"));
    el("acct").title = t("handoffHint");
    el("st").textContent = t(status.key, status.values);
    if (question) el("askt").textContent = t(question.key, question.values);
    el("usageRefresh").setAttribute("aria-label", t("usageRefresh"));
    renderConnection();
    renderUsage();
  }

  let usage = null;                // this tab's account
  let usageAll = {};               // every account read so far, by /u/N number
  let usageLoading = false;
  let usageWarned = false;
  function renderUsage() {
    const box = el("usageRows");
    box.replaceChildren();
    const rows = usage?.rows || [];
    if (!rows.length) {
      const d = document.createElement("div");
      d.className = "rs";
      d.textContent = usageLoading ? t("usageLoading") : usage?.error ? t("usageFailed") : t("usageNone");
      box.append(d);
    }
    for (const r of rows) {
      const d = document.createElement("div");
      const label = document.createElement("span");
      label.textContent = r.label;
      const pc = document.createElement("span");
      pc.className = "pc" + (r.percent >= 90 ? " hot" : "");
      pc.textContent = r.percent + "%";
      const rs = document.createElement("span");
      rs.className = "rs";
      rs.textContent = r.reset;
      rs.title = r.reset;
      d.append(label, pc, rs);
      box.append(d);
    }
    let tip = usage?.at ? t("usageUpdated", { time: new Date(usage.at).toLocaleString(language) }) : "";
    if (usage?.error) tip += (tip ? "\n" : "") + t("usageFailed");
    el("usage").title = tip;
    el("usageRefresh").disabled = usageLoading;
    renderAccounts();
  }
  function showUsage(value) {
    usageAll = value && !value.rows ? value : {}; // one record for all accounts before v1.3
    usage = usageAll[accountIndex()] || null;
    usageLoading = false;
    renderUsage();
    const full = (usage?.rows || []).some((r) => r.percent >= 100);
    if (full && !usageWarned) log("⚠️ " + t("usageLimit"));
    usageWarned = full;
  }
  async function refreshUsage(force) {
    if (isUsagePage()) return;
    if (force) {
      usageLoading = true;
      renderUsage();
      setTimeout(() => { if (usageLoading) { usageLoading = false; renderUsage(); } }, 35000);
    }
    const r = await bg({ type: "usage", force, account: accountIndex() });
    if (!r.ok) { usageLoading = false; renderUsage(); log("⚠️ " + t("usageFailed") + " " + (r.error || "")); }
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
    el("always").style.display = call ? "" : "none";
    el("skip").style.display = call ? "" : "none";
    el("ask").classList.add("show");
    root.querySelector(".p").classList.remove("min");
    notify(call ? t("waiting", { tool: call.tool }) : t(key, values), call ? short(call.args) : "");
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
  el("nt").onchange = (e) => { state.notify = e.target.value; save(); };
  el("ntest").onclick = async () => {
    const r = await sendNotice(t("notifyReply"), t("notifyTestBody"));
    if (r.ok) setStatus("notifyTestSent");
    else setStatus("notifyFailed", { detail: r.error }, true);
  };
  el("reload").onclick = async () => {
    if (state.busy) return setStatus("busy", {}, true);
    await bg({ type: "reload" });
    setTimeout(() => location.reload(), 800);
  };
  el("usageRefresh").onclick = () => refreshUsage(true);
  el("run").onclick = () => answer("run");
  el("always").onclick = () => answer("always");
  el("skip").onclick = () => answer("skip");
  el("halt").onclick = () => { state.abort = true; answer("stop"); };
  el("stop").onclick = () => { state.abort = true; state.steps = 0; stall.retries = 0; stall.gaveUp = false; answer("stop"); setStatus("stopped"); };
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
  el("handoff").onclick = startHandoff;
  el("acct").onchange = (e) => { handoffTo = Number(e.target.value); chrome.storage.local.set({ handoffTo }); };
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
    if (area === "local" && changes.usage) showUsage(changes.usage.newValue);
    if (area === "local" && changes.accounts) { accounts = changes.accounts.newValue?.list || []; renderAccounts(); }
    if (area === "local" && changes.handoffTo) { handoffTo = changes.handoffTo.newValue ?? null; renderAccounts(); }
    if (area === "local" && changes.language) {
      applyLanguage(geminiForgeLanguage(changes.language.newValue || chrome.i18n.getUILanguage()));
      health();
    }
  });
  const settingsReady = chrome.storage.local.get(["enabled", "mode", "autoPrompt", "notify", "language", "usage", "handoffTo", "accounts"]).then(async v => {
    accounts = v.accounts?.list || [];
    handoffTo = v.handoffTo ?? null;
    if (!isUsagePage() && !(Date.now() - v.accounts?.at < ACCOUNTS_AGE)) findAccounts();
    showUsage(v.usage);
    if (typeof v.enabled === "boolean") state.enabled = v.enabled;
    if (typeof v.autoPrompt === "boolean") state.autoPrompt = v.autoPrompt;
    if (Object.hasOwn(NOTIFY, v.notify)) state.notify = v.notify;
    else if (typeof v.notify === "boolean") state.notify = v.notify ? "away" : "off"; // the old on/off checkbox
    if (MODES.includes(v.mode)) state.mode = v.mode;
    if (!state.enabled) setStatus("disabled");
    await applyLanguage(geminiForgeLanguage(v.language || chrome.i18n.getUILanguage()));
    document.documentElement.appendChild(host);
  });
  health();
  setInterval(health, 5000);
  if (isUsagePage()) reportUsage();
  else settingsReady.then(() => { refreshUsage(false); resumeHandoff(); });
  // Not setInterval: sleep() keeps the one-second pace while the tab is hidden.
  (async () => {
    for (;;) {
      await sleep(1000);
      tick().catch((e) => console.error("[GeminiForge]", e));
    }
  })();
})();
