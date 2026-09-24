// Linggen API client — the session, model and skill-cloud endpoints every
// skill page uses. Served by the engine at /shared/api.js so skills stop
// carrying drifting copies; a skill keeps its own module only for calls that
// are its alone, importing these from here.
//
// Same-origin by design: a page served at /apps/<skill>/… reaches the engine
// that served it (loopback, or the tunnel that proxied it).

const API_BASE = '';

export async function fetchModels() {
  const res = await fetch(`${API_BASE}/api/models`);
  if (!res.ok) throw new Error('Failed to fetch models');
  return res.json();
}

export async function fetchDefaultModel() {
  const res = await fetch(`${API_BASE}/api/config`);
  if (!res.ok) return null;
  const config = await res.json();
  const defaults = config.routing?.default_models;
  return defaults && defaults.length > 0 ? defaults[0] : null;
}

export async function createSession(title, skill) {
  const res = await fetch(`${API_BASE}/api/sessions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ title, skill }),
  });
  if (!res.ok) throw new Error(`Failed to create session: ${res.status}`);
  return res.json();
}

export async function listSkillSessions(skill) {
  const res = await fetch(`${API_BASE}/api/skill-sessions?skill=${encodeURIComponent(skill)}`);
  if (!res.ok) return [];
  const data = await res.json();
  return data.sessions || [];
}

/// The day's chat to pick up, from the list the engine gives. `updated_at` is
/// the transcript's own mtime, so a session that was created and never spoken
/// in carries its creation time — which is what a lost kickoff leaves behind.
/// That is not a reopened day: it is passed over, and the newest session that
/// holds a conversation is the day's — if it began less than a day ago.
export function pickResumable(sessions, nowSec = Date.now() / 1000, maxAgeHours = 24) {
  const spoken = (sessions || []).filter(s => (s.updated_at || 0) > (s.created_at || 0));
  if (!spoken.length) return null;
  spoken.sort((a, b) => (b.created_at || 0) - (a.created_at || 0));
  const newest = spoken[0];
  return (nowSec - (newest.created_at || 0)) / 3600 < maxAgeHours ? newest.id : null;
}

/// `keepalive` lets a pagehide cleanup finish after the page is gone.
export async function removeSkillSession(skill, sessionId, { keepalive = false } = {}) {
  const res = await fetch(`${API_BASE}/api/skill-sessions`, {
    method: 'DELETE',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ skill, session_id: sessionId }),
    keepalive,
  });
  if (!res.ok) throw new Error(`Failed to delete session: ${res.status}`);
}

export async function fetchSessionMessages(skill, sessionId) {
  const params = new URLSearchParams({ skill, session_id: sessionId });
  const res = await fetch(`${API_BASE}/api/skill-sessions/state?${params}`);
  if (!res.ok) return [];
  const data = await res.json();
  return data.messages || [];
}

/// The skill's cloud as the engine sees it: `{signed_in, meter}` — the
/// meter is the window's last reading (`size, used, left, refill_at`) or
/// null when signed out. null when the skill declares no cloud.
export async function fetchCloud(skill) {
  const res = await fetch(`${API_BASE}/api/skill-cloud/${encodeURIComponent(skill)}`);
  // An engine without the route answers the web app's index page, 200.
  if (!res.ok || !(res.headers.get('content-type') || '').includes('json')) return null;
  return res.json();
}

/// Bring the save into step with the account's copy. Answers
/// `{done, version}`; null when signed out or the site is out of reach.
export async function syncCloud(skill) {
  const res = await fetch(`${API_BASE}/api/skill-cloud/${encodeURIComponent(skill)}/sync`, { method: 'POST' });
  if (!res.ok) return null;
  return res.json();
}

/// Sign in to linggen.dev: the daemon opens the browser; resolves once the
/// account reports signed in, or false when the login window closes.
export async function signIn() {
  const res = await fetch(`${API_BASE}/api/account/login`, { method: 'POST' });
  const out = await res.json().catch(() => ({}));
  if (out && out.opened === false && out.url) window.open(out.url, '_blank', 'noopener');
  const until = Date.now() + 300_000;
  while (Date.now() < until) {
    await new Promise((r) => setTimeout(r, 2000));
    try {
      const acc = await (await fetch(`${API_BASE}/api/account`)).json();
      if (acc?.signed_in) return true;
    } catch { /* keep waiting */ }
  }
  return false;
}
