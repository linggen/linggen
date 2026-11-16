const q = new URLSearchParams(location.search);
const id = q.get("id");
const floor = q.get("floor");

document.getElementById("origin").textContent = q.get("origin") || "(unknown site)";
document.getElementById("summary").textContent = q.get("summary") || "";

if (floor) {
  const warn = document.getElementById("floorWarn");
  warn.hidden = false;
  warn.textContent = `Sensitive action (${floor}) — always confirmed, even on trusted sites.`;
  // The floor never persists trust; approval covers this single action only.
  document.getElementById("always").hidden = true;
  document.getElementById("once").textContent = "Allow this action";
}

function send(decision) {
  chrome.runtime.sendMessage({ t: "gate_decision", id, decision });
  window.close();
}

for (const decision of ["always", "once", "deny"]) {
  document.getElementById(decision).addEventListener("click", (e) => {
    if (e.isTrusted) send(decision);
  });
}
