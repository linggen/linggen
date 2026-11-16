// ISOLATED world, document_start. Bridges the MAIN-world page hook (x-hook.js)
// to the service worker — MAIN world can't use chrome.*, the isolated world can.
// Forwards only our tagged graphql captures.
window.addEventListener("message", (event) => {
  if (event.source !== window) return;
  const d = event.data;
  if (!d || d.source !== "linggen-x-hook") return;
  try {
    chrome.runtime.sendMessage({ type: "x-capture", op: d.op, data: d.data });
  } catch (e) {
    /* service worker not reachable — ignore */
  }
});
