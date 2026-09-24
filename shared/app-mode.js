// Linggen app-mode integration, served at /shared/app-mode.js.
//
// When a skill page is hosted by a Linggen app shell, the shell appends
// `?app_mode=1` to the page URL and posts window messages to drive chrome the
// page can't render itself (the native Settings menu item). Public web users
// don't get app_mode=1, so this whole module is a no-op for them.
//
// Protocol (shell -> skill):
//   { type: "linggen:show-settings" }   show the settings overlay
//   { type: "linggen:hide-settings" }   hide the settings overlay
//
// The overlay slides over the page without unmounting it, so page state
// survives. Esc and the close button dismiss back to the app. It loads the
// skill's own `settings.html` (resolved against the page, not this script).

(function () {
  const params = new URLSearchParams(window.location.search);
  if (params.get("app_mode") !== "1") return;

  let overlay = null;

  function ensureOverlay() {
    if (overlay) return overlay;
    overlay = document.createElement("div");
    overlay.className = "app-mode-overlay";
    overlay.innerHTML = `
      <div class="app-mode-overlay-bar">
        <span class="app-mode-overlay-title">Settings</span>
        <button class="app-mode-overlay-close" aria-label="Close settings">×</button>
      </div>
      <iframe class="app-mode-overlay-frame" src="about:blank" title="Settings"></iframe>
    `;
    overlay.querySelector(".app-mode-overlay-close").addEventListener("click", hide);
    document.body.appendChild(overlay);
    return overlay;
  }

  function show() {
    const o = ensureOverlay();
    // Reload the settings page on each open so dynamic data (account usage,
    // model state) is current — the overlay and its iframe persist between
    // opens, so without this it shows the first read.
    const frame = o.querySelector(".app-mode-overlay-frame");
    if (frame) frame.src = "settings.html";
    o.classList.add("visible");
  }

  function hide() {
    if (overlay) overlay.classList.remove("visible");
  }

  window.addEventListener("message", (e) => {
    // Only the same origin (the local daemon's shell) drives the overlay; any
    // embedded iframe or popup is ignored.
    if (e.origin !== window.location.origin) return;
    const msg = e.data;
    if (!msg || typeof msg !== "object") return;
    if (msg.type === "linggen:show-settings") show();
    if (msg.type === "linggen:hide-settings") hide();
  });

  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && overlay && overlay.classList.contains("visible")) {
      hide();
    }
  });
})();
