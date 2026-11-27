// Runs in the page's own world, where Gemini's Quill editor can be reached
// (content.js lives in the extension's isolated world and cannot).
//
// content.js asks for text to go into the chat editor with a
// "geminiforge-insert" event on the editor. Quill takes the whole text in one
// step; typing it in with execCommand lays the page out again for every line,
// which froze the tab for seconds on a long tool result. The event is
// cancelled only once the text is really in, so content.js knows whether it
// still has to type the text itself.
document.addEventListener("geminiforge-insert", (event) => {
  try {
    const { text, atStart } = JSON.parse(event.detail);
    const ed = event.target;
    const quill = [ed.parentElement, ed]
      .map((node) => node && window.Quill?.find?.(node))
      .find((q) => q && typeof q.insertText === "function" && typeof q.getLength === "function");
    if (!quill || typeof text !== "string" || !text) return;
    const length = quill.getLength();
    const at = atStart ? 0 : Math.max(0, length - 1); // in front of the editor's closing newline
    quill.insertText(at, text, "user");
    // Quill may store the text a little shorter (CRLF becomes LF), so any
    // growth counts as taken; a read-only editor does not grow at all.
    const grown = quill.getLength() - length;
    if (grown <= 0) return;
    quill.setSelection(at + grown, 0, "silent");
    event.preventDefault();
  } catch (_) { /* content.js types the text in instead */ }
});
