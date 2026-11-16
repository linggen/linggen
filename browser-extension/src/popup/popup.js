async function render() {
  const { bridgeStatus = "disconnected", modules = [] } = await chrome.storage.session.get(["bridgeStatus", "modules"]);
  const connected = bridgeStatus === "connected";
  document.getElementById("dot").classList.toggle("on", connected);
  document.getElementById("state").textContent = connected ? "Connected to daemon" : "Daemon not connected";
  document.getElementById("modules").textContent = modules.length
    ? "Modules: " + modules.map((m) => m.id).join(", ")
    : "";
  renderTrusted();
}

// Origins granted "Always allow" in the control gate; revocable here.
async function renderTrusted() {
  const { trusted_origins = {} } = await chrome.storage.local.get("trusted_origins");
  const box = document.getElementById("trusted");
  box.textContent = "";
  const origins = Object.keys(trusted_origins).sort();
  if (!origins.length) return;
  const title = document.createElement("div");
  title.className = "trusted-title";
  title.textContent = "Trusted sites (agent control)";
  box.appendChild(title);
  for (const origin of origins) {
    const row = document.createElement("div");
    row.className = "trusted-row";
    const label = document.createElement("span");
    label.textContent = origin;
    const remove = document.createElement("button");
    remove.textContent = "✕";
    remove.title = "Revoke";
    remove.addEventListener("click", async () => {
      delete trusted_origins[origin];
      await chrome.storage.local.set({ trusted_origins });
      renderTrusted();
    });
    row.append(label, remove);
    box.appendChild(row);
  }
}

chrome.storage.session.onChanged.addListener(render);
chrome.storage.local.onChanged.addListener(renderTrusted);
render();
