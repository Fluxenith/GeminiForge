import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const languagesSrc = fs.readFileSync(path.join(root, "extension", "languages.js"), "utf8");
const contentSrc = fs.readFileSync(path.join(root, "extension", "content.js"), "utf8");

function loadLanguages() {
  const ctx = {};
  vm.runInNewContext(languagesSrc, ctx);
  return ctx;
}

function loadContentHelpers(initialHref = "https://gemini.google.com/app") {
  let href = initialHref;
  const ctx = {
    URL,
    location: {
      get href() { return href; },
      set href(v) { href = v; },
      get pathname() { return new URL(href).pathname; },
    },
  };
  const extract = (pattern) => {
    const m = contentSrc.match(pattern);
    assert.ok(m, `Missing match for ${pattern}`);
    return m[0];
  };
  const snippets = [
    extract(/const READ_ONLY = new Set\(\[[\s\S]*?\]\);/),
    extract(/function repairJson\(s\) \{[\s\S]*?\n  \}/),
    extract(/function tryParse\(s\) \{[\s\S]*?\n  \}/),
    extract(/function scanObjects\(text\) \{[\s\S]*?\n  \}/),
    extract(/function normalize\(obj\) \{[\s\S]*?\n  \}/),
    extract(/function sameConversation\(baseUrl\) \{[\s\S]*?\n  \}/),
  ];
  const exported = vm.runInNewContext(
    `(() => { ${snippets.join("\n")}\n return { READ_ONLY, repairJson, tryParse, scanObjects, normalize, sameConversation }; })()`,
    ctx,
  );
  return {
    ...exported,
    setHref(next) { href = next; },
  };
}

const clone = (v) => JSON.parse(JSON.stringify(v));

test("geminiForgeLanguage maps locale tags consistently", () => {
  const { geminiForgeLanguage } = loadLanguages();
  assert.equal(geminiForgeLanguage("zh_TW"), "zh-TW");
  assert.equal(geminiForgeLanguage("zh-Hant-HK"), "zh-TW");
  assert.equal(geminiForgeLanguage("zh-CN"), "zh-CN");
  assert.equal(geminiForgeLanguage("zh_Hans_SG"), "zh-CN");
  assert.equal(geminiForgeLanguage("zh"), "zh-CN");
  assert.equal(geminiForgeLanguage("ja-JP"), "ja");
  assert.equal(geminiForgeLanguage("ko_KR"), "ko");
  assert.equal(geminiForgeLanguage("fr-FR"), "en");
});

test("READ_ONLY includes job_status, list_backups, read_window, ocr_window, and wait_for", () => {
  const { READ_ONLY } = loadContentHelpers();
  assert.equal(READ_ONLY.has("job_status"), true);
  assert.equal(READ_ONLY.has("list_backups"), true);
  assert.equal(READ_ONLY.has("read_window"), true);
  assert.equal(READ_ONLY.has("ocr_window"), true);
  assert.equal(READ_ONLY.has("wait_for"), true);
  assert.equal(READ_ONLY.has("click_text"), false);
  assert.equal(READ_ONLY.has("gui_batch"), false);
  assert.equal(READ_ONLY.has("type_element"), false);
  assert.equal(READ_ONLY.has("write_file"), false);
});

test("repairJson escapes control chars and strips trailing commas without corrupting strings", () => {
  const { tryParse } = loadContentHelpers();
  const raw = '{"gf_tool": "write_file", "args": {"path": "a.txt", "content": "a, ] b, }\nline2\tok",},}';
  const parsed = tryParse(raw);
  assert.equal(parsed.gf_tool, "write_file");
  assert.equal(parsed.args.content, "a, ] b, }\nline2\tok");
});

test("scanObjects continues past prose mentions and unclosed braces", () => {
  const { scanObjects, normalize } = loadContentHelpers();
  const proseBefore = 'Use "gf_tool" format like {unclosed then {"gf_tool": "read_file", "args": {"path": "a.py"}}';
  const found1 = scanObjects(proseBefore).flatMap(normalize);
  assert.deepStrictEqual(clone(found1), [{ tool: "read_file", args: { path: "a.py" } }]);

  const unclosedFirst = '{oops "gf_tool" {"gf_tool": "cd", "args": {"path": "."}}';
  const found2 = scanObjects(unclosedFirst).flatMap(normalize);
  assert.deepStrictEqual(clone(found2), [{ tool: "cd", args: { path: "." } }]);
});

test("normalize handles flat args and rejects array args", () => {
  const { normalize } = loadContentHelpers();
  assert.deepStrictEqual(clone(normalize({ gf_tool: " list_dir ", path: "." })), [{ tool: "list_dir", args: { path: "." } }]);
  assert.deepStrictEqual(clone(normalize({ gf_tool: "click", args: [1, 2], x: 5, y: 6 })), [{ tool: "click", args: { args: [1, 2], x: 5, y: 6 } }]);
});

// sleep() and waitFor() with a tab whose own timers run `throttle` times late.
function loadTimers({ hidden, throttle = 1, worker = true }) {
  const extract = (pattern) => {
    const m = contentSrc.match(pattern);
    assert.ok(m, `Missing match for ${pattern}`);
    return m[0];
  };
  const sent = [];
  const ctx = {
    Date, Promise,
    document: { hidden },
    setTimeout: (fn, ms) => setTimeout(fn, ms * throttle),
    bg: (msg) => {
      sent.push(msg);
      return worker ? new Promise((r) => setTimeout(() => r({ ok: true }), msg.ms)) : Promise.resolve({ ok: false });
    },
  };
  const snippets = [
    extract(/const localSleep = .*/),
    extract(/async function sleep\(ms\) \{[\s\S]*?\n  \}/),
    extract(/async function waitFor\(fn[\s\S]*?\n  \}/),
  ];
  const exported = vm.runInNewContext(`(() => { ${snippets.join("\n")}\n return { sleep, waitFor }; })()`, ctx);
  return { ...exported, sent };
}

test("sleep keeps its pace in a hidden tab by sleeping in the service worker", async () => {
  const hiddenTab = loadTimers({ hidden: true, throttle: 100 });
  let start = Date.now();
  await hiddenTab.sleep(30);
  assert.ok(Date.now() - start < 1000, "hidden tab slept in the worker");
  assert.deepStrictEqual(clone(hiddenTab.sent), [{ type: "sleep", ms: 30 }]);

  const visibleTab = loadTimers({ hidden: false });
  await visibleTab.sleep(10);
  assert.equal(visibleTab.sent.length, 0);

  // No worker (extension just reloaded): still waits the full time.
  const orphan = loadTimers({ hidden: true, worker: false });
  start = Date.now();
  await orphan.sleep(60);
  assert.ok(Date.now() - start >= 50);
});

test("waitFor checks once more after a sleep that overran the timeout", async () => {
  const { waitFor } = loadTimers({ hidden: false, throttle: 20 });
  let ready = false;
  setTimeout(() => { ready = true; }, 20);
  assert.equal(await waitFor(() => ready, 50, 10), true); // the 10 ms sleep took 200 ms
  assert.equal(await waitFor(() => false, 20, 5), null);
});

const pageSrc = fs.readFileSync(path.join(root, "extension", "page.js"), "utf8");

// insertText() in content.js together with page.js, against a fake Quill.
// `page` is false when page.js is not there (an older Chrome, or the page no
// longer exposes Quill); `readOnly` is an editor that takes no text.
function loadInsert({ page = true, quill = true, readOnly = false } = {}) {
  const extract = (pattern) => {
    const m = contentSrc.match(pattern);
    assert.ok(m, `Missing match for ${pattern}`);
    return m[0];
  };
  const calls = [];
  const listeners = [];
  const state = { text: "typed\n", caret: null };
  const instance = {
    getLength: () => state.text.length,
    insertText(at, text, source) {
      calls.push(`quill@${at}:${source}`);
      // Like Quill, line ends are stored as "\n" whatever came in.
      if (!readOnly) state.text = state.text.slice(0, at) + text.replace(/\r\n?/g, "\n") + state.text.slice(at);
    },
    setSelection(at) { state.caret = at; },
  };
  const container = {};
  const ed = {
    parentElement: container,
    focus() { calls.push("focus"); },
    dispatchEvent(event) {
      event.target = ed;
      for (const listener of listeners) listener(event);
      return !event.defaultPrevented;
    },
  };
  class CustomEvent {
    constructor(type, init) { this.type = type; this.detail = init.detail; this.defaultPrevented = false; }
    preventDefault() { this.defaultPrevented = true; }
  }
  const document = {
    addEventListener(type, fn) { if (type === "geminiforge-insert") listeners.push(fn); },
    createRange: () => ({ selectNodeContents() {}, collapse() {} }),
    execCommand(_command, _ui, text) { calls.push("exec"); state.text = state.text.slice(0, -1) + text + "\n"; },
  };
  const window = {
    getSelection: () => ({ removeAllRanges() {}, addRange() {} }),
    // Quill.find gives the instance for the container and a blot for the editor itself.
    Quill: quill ? { find: (node) => (node === container ? instance : node === ed ? { domNode: ed } : null) } : undefined,
  };
  const ctx = { editor: () => ed, t: (key) => key, window, document, CustomEvent, JSON, Math };
  if (page) vm.runInNewContext(pageSrc, ctx);
  const insertText = vm.runInNewContext(`(() => {
    ${extract(/function caretAt\(ed, atStart\) \{[\s\S]*?\n  \}/)}
    ${extract(/function insertText\(text, atStart = false\) \{[\s\S]*?\n  \}/)}
    return insertText;
  })()`, ctx);
  return { insertText, state, calls };
}

test("insertText goes through Quill in one step and types only when that is not possible", () => {
  const text = "[GeminiForge result]\n\n--- 1. run ---\n1. kept as text\n";

  const end = loadInsert();
  end.insertText(text);
  assert.equal(end.state.text, "typed" + text + "\n");                 // in front of the closing newline
  assert.equal(end.state.caret, 5 + text.length);
  assert.deepStrictEqual(end.calls, ["focus", "quill@5:user"]);        // never typed: no freeze

  const start = loadInsert();
  start.insertText(text, true);
  assert.equal(start.state.text, text + "typed\n");
  assert.deepStrictEqual(start.calls, ["focus", "quill@0:user"]);

  // The prompt files can have CRLF line ends: Quill stores fewer characters
  // than it was given, which must not be taken for "nothing went in".
  const crlf = loadInsert();
  crlf.insertText("a\r\nb\r\nc");
  assert.equal(crlf.state.text, "typeda\nb\nc\n");
  assert.equal(crlf.state.caret, 5 + 5);
  assert.deepStrictEqual(crlf.calls, ["focus", "quill@5:user"]);      // not typed a second time

  // Every way of not getting the text in ends with it typed once, not twice.
  for (const options of [{ page: false }, { quill: false }]) {
    const typed = loadInsert(options);
    typed.insertText(text);
    assert.equal(typed.state.text, "typed" + text + "\n");
    assert.deepStrictEqual(typed.calls, ["focus", "exec"]);
  }
  const readOnly = loadInsert({ readOnly: true });
  readOnly.insertText(text);
  assert.deepStrictEqual(readOnly.calls, ["focus", "quill@5:user", "exec"]);
});

test("the manifest runs page.js in the page's own world", () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(root, "extension", "manifest.json"), "utf8"));
  const entry = manifest.content_scripts.find((x) => x.js.includes("page.js"));
  assert.equal(entry?.world, "MAIN");
  assert.deepStrictEqual(entry.matches, ["https://gemini.google.com/*"]);
});

test("a reply is stalled only after two minutes of generating with no change", () => {
  const extract = (pattern) => {
    const m = contentSrc.match(pattern);
    assert.ok(m, `Missing match for ${pattern}`);
    return m[0];
  };
  const { stalled, stall } = vm.runInNewContext(`(() => {
    ${extract(/const STALL_MS = .*/)}
    ${extract(/let stall = .*/)}
    ${extract(/function stalled\(sig, generating, now\) \{[\s\S]*?\n  \}/)}
    return { stalled, stall };
  })()`);
  assert.equal(stalled("3|0", true, 1000), false);       // starts the clock
  assert.equal(stalled("3|0", true, 120000), false);
  assert.equal(stalled("3|0", true, 121000), true);
  assert.equal(stalled("3|40", true, 122000), false);    // text arrived: clock restarts
  assert.equal(stalled("3|40", true, 250000), true);
  assert.equal(stalled("3|40", false, 251000), false);   // not generating
  assert.equal(stalled("3|40", true, 252000), false);
  stall.gaveUp = true;
  assert.equal(stalled("3|40", true, 999000), false);    // already reported to the user
});

function loadHandoff() {
  const extract = (pattern) => {
    const m = contentSrc.match(pattern);
    assert.ok(m, `Missing match for ${pattern}`);
    return m[0];
  };
  return vm.runInNewContext(`(() => {
    ${extract(/const HANDOFF_BUDGET = .*/)}
    ${extract(/const HANDOFF_RECENT = .*/)}
    ${extract(/const REQUEST_MARK = .*/)}
    ${extract(/const HANDOFF_MARKS = .*/)}
    ${extract(/const accountIndex = .*/)}
    ${extract(/function pageAccount\(html\) \{[\s\S]*?\n  \}/)}
    ${extract(/function stripPrompt\(text, prompt = ""\) \{[\s\S]*?\n  \}/)}
    ${extract(/const HANDOFF_TYPED = .*/)}
    ${extract(/function packHandoff\(turns, budget = HANDOFF_BUDGET, spare = HANDOFF_TYPED\) \{[\s\S]*?\n  \}/)}
    return { accountIndex, pageAccount, stripPrompt, packHandoff, HANDOFF_MARKS };
  })()`, { location: { pathname: "/app" } });
}

test("pageAccount reads the address and account number out of a Gemini page", () => {
  const { pageAccount } = loadHandoff();
  const page = 'window.WIZ_global_data = {"QrtxK":"3","oPEP7c":"someone@example.com","x":"help@example.org"};';
  assert.deepStrictEqual(clone(pageAccount(page)), { email: "someone@example.com", index: 3 });
  // Without the known keys: the first value that is exactly an address.
  assert.deepStrictEqual(clone(pageAccount('{"a":"mail me at x@y.z","b":"other@example.com"}')), { email: "other@example.com", index: null });
  assert.deepStrictEqual(clone(pageAccount("<html>signed out</html>")), { email: "", index: null });
});

test("accountIndex reads the /u/N prefix and defaults to the first account", () => {
  const { accountIndex } = loadHandoff();
  assert.equal(accountIndex(), 0);
  assert.equal(accountIndex("/app/abc123"), 0);
  assert.equal(accountIndex("/u/2/app"), 2);
  assert.equal(accountIndex("/u/1"), 1);
  assert.equal(accountIndex("/gem/u/3/app"), 0);
});

test("stripPrompt leaves only the request from a chat's first message", () => {
  const { stripPrompt, HANDOFF_MARKS } = loadHandoff();
  const prompt = "[GeminiForge tools]\nUse a json block.\n\n## Example\nYou: ok";
  for (const mark of ["My request: ", "依頼内容：", "요청 사항: ", "我的需求："]) {
    assert.equal(stripPrompt(`${prompt}\n\n${mark}fix the build\nand test it`, prompt), "fix the build\nand test it");
  }
  // Inserted by hand in front of the text; the page changed the whitespace.
  assert.equal(stripPrompt("[GeminiForge tools]\n\nUse a  json block.\n## Example\n\nYou: ok\nfix the build", prompt), "fix the build");
  assert.equal(stripPrompt("  just a question  ", prompt), "just a question");
  // A chat that itself began with a handoff gives back the task inside it.
  const pack = `${prompt}\n\n[GeminiForge handoff] intro\n\n${HANDOFF_MARKS[0]}\nfix the build\n\n${HANDOFF_MARKS[1]}\n[Gemini]\nMy request: x`;
  assert.equal(stripPrompt(pack, prompt), "fix the build");
});

test("packHandoff keeps the newest messages whole, shortens older ones and counts what is left out", () => {
  const { packHandoff } = loadHandoff();
  const turns = Array.from({ length: 10 }, (_, i) => ({ role: i % 2 ? "user" : "model", text: `${i % 2 ? "[GeminiForge result] " : ""}${i}:`.padEnd(1000, "x") }));
  const all = packHandoff(turns, 100000);
  assert.equal(all.skipped, 0);
  assert.equal(all.kept.length, 10);
  assert.equal(all.kept[9].text, turns[9].text);                 // recent: whole
  assert.equal(all.kept[4].text, turns[4].text);
  assert.equal(all.kept[3].text, turns[3].text.slice(0, 400) + " …[+600]"); // older: cut
  assert.equal(all.kept[3].role, "user");

  const tight = packHandoff(turns, 3500);
  assert.equal(tight.kept.length, 3);
  assert.equal(tight.skipped, 7);
  assert.equal(tight.kept[0].text, turns[7].text);
  assert.deepStrictEqual(clone(packHandoff([], 100)), { kept: [], skipped: 0 });
});

test("packHandoff keeps what the user typed from further back than tool results", () => {
  const { packHandoff } = loadHandoff();
  const result = (i) => ({ role: "user", text: `[GeminiForge result] ${i}`.padEnd(300, "x") });
  const reply = (i) => ({ role: "model", text: `reply ${i}`.padEnd(300, "x") });
  const turns = [
    { role: "user", text: "use the blue theme" }, reply(1), result(2), reply(3),
    { role: "user", text: "and skip the tests" }, reply(5), result(6), reply(7),
  ];
  const tight = packHandoff(turns, 700);
  assert.deepStrictEqual(clone(tight.kept.map((x) => x.text.slice(0, 18))),
    ["use the blue theme", "and skip the tests", result(6).text.slice(0, 18), "reply 7xxxxxxxxxxx"]);
  assert.equal(tight.skipped, 4);
  // The allowance for typed messages runs out too.
  const none = packHandoff(turns, 700, 20);
  assert.deepStrictEqual(clone(none.kept.map((x) => x.text.slice(0, 18))),
    ["and skip the tests", result(6).text.slice(0, 18), "reply 7xxxxxxxxxxx"]);
  assert.equal(none.skipped, 5);
  // An older typed message is shortened too, but less than the rest.
  const long = packHandoff([{ role: "user", text: "y".repeat(2000) }, ...Array.from({ length: 6 }, (_, i) => reply(i))], 100000);
  assert.equal(long.kept[0].text, "y".repeat(1500) + " …[+500]");
});

test("every locale has the same message keys", () => {
  const dir = path.join(root, "extension", "_locales");
  const keysOf = (locale) => Object.keys(JSON.parse(fs.readFileSync(path.join(dir, locale, "messages.json"), "utf8"))).sort();
  const en = keysOf("en");
  for (const key of ["stalled", "stallNudge", "stallGiveUp"]) assert.ok(en.includes(key), key);
  for (const locale of fs.readdirSync(dir)) assert.deepStrictEqual(keysOf(locale), en, locale);
});

test("sameConversation treats /app -> /app/id as same chat and other chats as different", () => {
  const h = loadContentHelpers("https://gemini.google.com/app");
  const startUrl = "https://gemini.google.com/app";
  h.setHref("https://gemini.google.com/app/abc123");
  assert.equal(h.sameConversation(startUrl), true);

  const chatA = "https://gemini.google.com/app/abc123";
  h.setHref("https://gemini.google.com/app/def456");
  assert.equal(h.sameConversation(chatA), false);

  h.setHref("https://gemini.google.com/app");
  assert.equal(h.sameConversation(chatA), false);
});
