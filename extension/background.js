// Relays extension messages to packaged resources and the local server.
importScripts("languages.js");
const SERVER = "http://127.0.0.1:18765";

// Desktop notifications are tagged with the Gemini tab they came from, so a
// click can bring that tab back and returning to the tab can clear them.
const notifyPrefix = (tabId) => `gf:${tabId}:`;
async function clearTabNotifications(tabId) {
  const all = await chrome.notifications.getAll();
  await Promise.all(Object.keys(all).filter((id) => id.startsWith(notifyPrefix(tabId))).map((id) => chrome.notifications.clear(id)));
}
chrome.notifications.onClicked.addListener(async (id) => {
  chrome.notifications.clear(id);
  const tabId = Number(id.split(":")[1]);
  if (!tabId) return;
  try {
    const tab = await chrome.tabs.update(tabId, { active: true });
    await chrome.windows.update(tab.windowId, { focused: true });
  } catch (_) { /* the tab was closed */ }
});

// Usage limits. Gemini has no usage API and its /usage page refuses to be
// framed, so the page is opened in a background tab; GeminiForge's content
// script there reads it, reports with "usageData", and the tab is closed.
// Each signed-in Google account (gemini.google.com/u/N) has its own limits,
// so the numbers are kept per account number.
const accountOf = (value) => Math.min(Math.max(Math.trunc(Number(value)) || 0, 0), 9);
const USAGE_MAX_AGE = 5 * 60 * 1000;   // automatic refreshes skip fresher data
const USAGE_TIMEOUT = 30 * 1000;
let usageLock = Promise.resolve();   // several Gemini tabs may ask at once
function refreshUsage(windowId, force, account) {
  usageLock = usageLock.then(() => openUsageTab(windowId, force, account)).catch(() => {});
  return usageLock;
}
async function loadUsage() {
  const { usage } = await chrome.storage.local.get("usage");
  return usage && !usage.rows ? usage : {};   // one record for all accounts before v1.3
}
async function openUsageTab(windowId, force, account) {
  const known = (await loadUsage())[account];
  if (!force && known?.at && Date.now() - known.at < USAGE_MAX_AGE) return;
  // The worker may have restarted, so the open tab is kept in session storage.
  const { usageTab } = await chrome.storage.session.get("usageTab");
  if (usageTab && Date.now() - usageTab.since < USAGE_TIMEOUT) return;
  if (usageTab) await closeUsageTab(usageTab, "timeout");
  const tab = await chrome.tabs.create({ url: `https://gemini.google.com/u/${account}/usage`, active: false, ...(windowId ? { windowId } : {}) });
  await chrome.storage.session.set({ usageTab: { id: tab.id, since: Date.now(), account } });
  setTimeout(async () => {
    const { usageTab: open } = await chrome.storage.session.get("usageTab");
    if (open?.id === tab.id) closeUsageTab(open, "timeout");
  }, USAGE_TIMEOUT);
}
async function closeUsageTab(usageTab, error) {
  await chrome.storage.session.remove("usageTab");
  try { await chrome.tabs.remove(usageTab.id); } catch (_) { /* already closed */ }
  if (error) await saveUsage(accountOf(usageTab.account), [], error);
}
async function saveUsage(account, rows, error = "") {
  const all = await loadUsage();
  // A failed read keeps the last numbers and their time.
  all[account] = rows.length ? { rows, at: Date.now(), error: "" } : { rows: all[account]?.rows || [], at: all[account]?.at || 0, error };
  await chrome.storage.local.set({ usage: all });
}

// Handoff to another Google account: the pack waits here for the content
// script of the tab opened for it, and only that tab can take it, once.
const HANDOFF_TTL = 10 * 60 * 1000;   // long enough to sign in on the way

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  (async () => {
    try {
      if (msg.type === "handoff") {
        const to = accountOf(msg.to);
        const windowId = sender.tab?.windowId;
        const tab = await chrome.tabs.create({ url: `https://gemini.google.com/u/${to}/app`, active: true, ...(windowId ? { windowId } : {}) });
        await chrome.storage.session.set({ handoff: { tabId: tab.id, to, text: String(msg.text || ""), at: Date.now() } });
        sendResponse({ ok: true });
      } else if (msg.type === "handoffTake") {
        const { handoff } = await chrome.storage.session.get("handoff");
        const mine = !!handoff && handoff.tabId === sender.tab?.id && Date.now() - handoff.at < HANDOFF_TTL;
        if (mine) await chrome.storage.session.remove("handoff");
        sendResponse({ ok: true, handoff: mine ? { to: handoff.to, text: handoff.text } : null });
      } else if (msg.type === "usage") {
        await refreshUsage(sender.tab?.windowId, !!msg.force, accountOf(msg.account));
        sendResponse({ ok: true });
      } else if (msg.type === "usageData") {
        const rows = Array.isArray(msg.rows) ? msg.rows.slice(0, 8) : [];
        // Filed under the account the page really shows: asking for one that
        // is not signed in lands on another account's page.
        await saveUsage(accountOf(msg.account), rows, rows.length ? "" : String(msg.error || "notFound"));
        const { usageTab } = await chrome.storage.session.get("usageTab");
        if (usageTab && usageTab.id === sender.tab?.id) await closeUsageTab(usageTab);
        sendResponse({ ok: true });
      } else if (msg.type === "notify") {
        const tabId = sender.tab?.id ?? 0;
        // One notification per tab: a newer event replaces the older one.
        await clearTabNotifications(tabId);
        await chrome.notifications.create(notifyPrefix(tabId) + Date.now(), {
          type: "basic",
          iconUrl: "icons/icon128.png",
          title: String(msg.title || "GeminiForge").slice(0, 200),
          message: String(msg.message || "").slice(0, 500),
          priority: 1,
        });
        sendResponse({ ok: true });
      } else if (msg.type === "clearNotify") {
        if (sender.tab?.id) await clearTabNotifications(sender.tab.id);
        sendResponse({ ok: true });
      } else if (msg.type === "sleep") {
        // A hidden tab's own timers are throttled; this one is not.
        await new Promise((r) => setTimeout(r, Math.min(Math.max(Number(msg.ms) || 0, 0), 25000)));
        sendResponse({ ok: true });
      } else if (msg.type === "reload") {
        sendResponse({ ok: true });
        setTimeout(() => chrome.runtime.reload(), 100);
      } else if (msg.type === "health") {
        const r = await fetch(`${SERVER}/health`);
        const health = await r.json();
        // Older servers do not advertise the shared desktop language preference.
        if (health.languageSync) {
          const prefs = await chrome.storage.local.get(["language", "desktopLanguage"]);
          const language = geminiForgeLanguage(prefs.language || chrome.i18n.getUILanguage());
          if (prefs.desktopLanguage !== language) {
            const saved = await fetch(`${SERVER}/language`, {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({ language }),
            });
            if (saved.ok && (await saved.json()).ok) await chrome.storage.local.set({ desktopLanguage: language });
          }
        }
        sendResponse(health);
      } else if (msg.type === "tool") {
        const r = await fetch(`${SERVER}/tool`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ tool: msg.tool, args: msg.args || {} }),
        });
        sendResponse(await r.json());
      } else if (msg.type === "messages" || msg.type === "prompt") {
        const language = geminiForgeLanguage(msg.language);
        // macOS and Linux have the file and command tools only, so they get their own prompt.
        const unix = msg.os === "macos" || msg.os === "linux";
        const path = msg.type === "messages"
          ? `_locales/${language.replace("-", "_")}/messages.json`
          : `prompts/${unix ? "unix/" : ""}${language}.md`;
        const r = await fetch(chrome.runtime.getURL(path));
        if (!r.ok) throw new Error(path);
        sendResponse(msg.type === "messages" ? { ok: true, messages: await r.json() } : { ok: true, text: await r.text() });
      }
    } catch (e) {
      const offline = msg.type === "tool" || msg.type === "health";
      sendResponse({ ok: false, offline, error: String(e.message) });
    }
  })();
  return true;
});
