// Relays extension messages to packaged resources and the local server.
importScripts("languages.js");
const SERVER = "http://127.0.0.1:18765";

chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  (async () => {
    try {
      if (msg.type === "reload") {
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
        const path = msg.type === "messages"
          ? `_locales/${language.replace("-", "_")}/messages.json`
          : `prompts/${language}.md`;
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
