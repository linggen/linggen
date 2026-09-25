// Chat bridge — mounts the full Linggen chat panel as an iframe. Served by the
// engine at /shared/chat-bridge.js; every skill page uses this one copy.
// The iframe loads /embed?skill=… (the complete chat UI: markdown, tool
// activity, permissions, plans, subagents); this module talks to it over
// postMessage with the chat's own origin only, never '*'.
//
// `lazy: true` defers the session and the iframe until the first send() — a
// page costs no session and no LLM until the user acts. Until then a light
// local panel shows the welcome and a working input.
//
// Import for its side effect (sets window.LinggenUI) or take `mount` directly:
//   import '/shared/chat-bridge.js';
//   import { mount } from '/shared/chat-bridge.js';

// Absolute, like the pages' own import: the linggen.dev relay inlines modules
// by their resolved path, so '/shared/api.js' and '/shared/./api.js' would be
// bundled twice and clash.
import { createSession, removeSkillSession } from '/shared/api.js';

/**
 * Mount a chat iframe into the given element.
 * @param {HTMLElement} el
 * @param {{
 *   skillName: string,
 *   agentId?: string,
 *   modelId?: string,
 *   title?: string,
 *   sessionId?: string,
 *   placeholder?: string,
 *   lazy?: boolean,
 *   deleteOnLeave?: boolean,
 *   onSessionCreated?: (sid: string) => void,
 *   onStreamToken?: (fullText: string, info: { agent: string, own: boolean }) => void,
 *   onStreamEnd?: (text: string, info: { agent: string, own: boolean }) => void,
 *   guestStreams?: boolean,
 *   onContentBlock?: (payload: { phase: string, tool?: string, args?: string, blockId?: string, output?: string }) => void,
 *   onActivity?: (payload: { sessionId: string, kind: 'turn_start' | 'thinking' | 'tool' }) => void,
 *   onSendFailed?: (payload: { text: string }) => void,
 *   onConnectionChange?: (status: 'connected' | 'reconnecting' | 'disconnected') => void,
 *   onSkillEvent?: (event: string, payload: any) => void,
 * }} options
 * @returns {Promise<ChatInstance>}
 */
// One app chat holds the skill's own agent (`agentId`) and a guest addressed
// by `@name` (Yinyue). onStreamToken / onStreamEnd hear the skill's own
// agent's turns only — a guest's line never ends the page's turn. A page that
// wants her lines too passes `guestStreams: true`; each call then says whose
// (`info.agent`, `info.own`).
export async function mount(el, options) {
  const { skillName, onSessionCreated, onStreamToken, onStreamEnd } = options;
  const lazy = !!options.lazy;

  let modelId = options.modelId || '';
  let sessionId = options.sessionId || null;
  const streamBuffers = new Map(); // agent → its turn's text so far
  let iframe = null;
  let mounting = null; // ensureMounted's promise — mount exactly once
  let embedAlive = false; // first inbound event from the embed = transport up
  const outbox = []; // posts queued until the embed is provably alive
  const localLog = []; // messages shown before the iframe exists — replayed in
  // Assume healthy until the iframe says otherwise: a page that never hears a
  // connection event (older embed build) behaves exactly as before.
  let connectionStatus = 'connected';

  // Remote mode: a meta tag injected by the relay's connect page.
  const instanceMeta = document.querySelector('meta[name="linggen-instance"]');
  const relayOrigin = (document.querySelector('meta[name="linggen-relay-origin"]') || {}).content
    || window.location.origin;
  // The one origin the chat speaks from and is spoken to at.
  const chatOrigin = instanceMeta
    ? new URL(relayOrigin, window.location.href).origin
    : window.location.origin;

  function buildSrc(sid) {
    const p = new URLSearchParams({ skill: skillName, session: sid, hide_toolbar: '1' });
    if (modelId) p.set('model', modelId);
    if (options.agentId) p.set('agent', options.agentId);
    if (instanceMeta) {
      // Remote: the relay's connect page establishes its own WebRTC.
      const instanceId = instanceMeta.getAttribute('content') || '';
      return `${relayOrigin}/app/connect/${instanceId}?${p.toString()}&entry=embed`;
    }
    return `/embed?${p.toString()}`;
  }

  // ── Local placeholder (lazy mode): welcome bubbles + a real input ──
  let localPanel = null;
  function renderLocalMessage(role, text) {
    if (!localPanel) return;
    const bubble = document.createElement('div');
    bubble.style.cssText = 'margin:10px 12px;padding:10px 12px;border-radius:10px;font-size:13px;line-height:1.5;' +
      (role === 'assistant'
        ? 'background:rgba(255,255,255,.06);color:#e0e0e0;'
        : 'background:#0f3460;color:#fff;margin-left:40px;');
    bubble.textContent = text;
    localPanel.querySelector('.local-log').appendChild(bubble);
  }
  if (lazy) {
    localPanel = document.createElement('div');
    localPanel.style.cssText = 'display:flex;flex-direction:column;height:100%;';
    localPanel.innerHTML = `
      <div class="local-log" style="flex:1;overflow-y:auto;"></div>
      <form class="local-input" style="display:flex;gap:8px;padding:10px 12px;">
        <input type="text" style="flex:1;padding:8px 10px;border-radius:8px;border:1px solid rgba(255,255,255,.15);background:rgba(255,255,255,.06);color:inherit;font-size:13px;outline:none;">
        <button type="submit" style="padding:8px 14px;border-radius:8px;border:none;background:#e94560;color:#fff;font-size:13px;cursor:pointer;">Send</button>
      </form>`;
    localPanel.querySelector('input').placeholder = options.placeholder || 'Type a message...';
    localPanel.querySelector('form').addEventListener('submit', (e) => {
      e.preventDefault();
      const input = localPanel.querySelector('input');
      const text = input.value.trim();
      if (!text) return;
      input.value = '';
      send(text);
    });
    el.appendChild(localPanel);
  }

  // Posts made before the embed's listener and transport are up are silently
  // lost — queue them until it shows life (any event) or a grace period ends.
  function post(msg) {
    if (embedAlive && iframe) { iframe.contentWindow?.postMessage(msg, chatOrigin); return; }
    outbox.push(msg);
  }
  function flushOutbox() {
    if (!iframe) return;
    embedAlive = true;
    while (outbox.length) iframe.contentWindow?.postMessage(outbox.shift(), chatOrigin);
  }
  function armGracePeriod() {
    setTimeout(() => { if (!embedAlive) flushOutbox(); }, 4000);
  }

  // An older embed names no agent: its stream is the page's own, as before.
  function streamInfo(payload) {
    return { agent: payload?.agent || '', own: payload?.own !== false };
  }
  function heard(info) { return info.own || !!options.guestStreams; }

  const handlers = {
    stream_token(payload) {
      const info = streamInfo(payload);
      const text = (streamBuffers.get(info.agent) || '') + (payload?.text || '');
      streamBuffers.set(info.agent, text);
      if (onStreamToken && heard(info)) onStreamToken(text, info);
    },
    stream_end(payload) {
      const info = streamInfo(payload);
      // Prefer the raw tokens over payload.text (may be rendered/stripped).
      const text = streamBuffers.get(info.agent) || payload?.text || '';
      streamBuffers.delete(info.agent);
      if (onStreamEnd && heard(info)) onStreamEnd(text, info);
    },
    content_block(payload) { options.onContentBlock?.(payload); },
    // The agent is working though nothing streams — at most one per ~5 s.
    activity(payload) { options.onActivity?.(payload); },
    send_failed(payload) { options.onSendFailed?.(payload); },
    connection(payload) {
      // The iframe owns the transport; the page sees the agent through it.
      connectionStatus = payload?.status || 'disconnected';
      options.onConnectionChange?.(connectionStatus);
    },
    session_created(payload) {
      if (!payload?.sessionId) return;
      sessionId = payload.sessionId;
      tellStages();
      if (onSessionCreated) onSessionCreated(sessionId);
    },
  };

  // ── Yinyue's stage on this page ──
  // A stage framed here (the engine's pet view, `?pet=1&stage=1`) is told
  // this chat's session: while it holds her, what the user says to her
  // lands in this chat — one conversation per app. Only our own origin's
  // frames hear it; the chat's own iframe is not a stage.
  const STAGE_MSG = 'linggen-app-chat';
  function tellStages() {
    for (const f of document.querySelectorAll('iframe')) {
      if (f === iframe) continue;
      try { f.contentWindow?.postMessage({ type: STAGE_MSG, session: sessionId }, window.location.origin); } catch { /* not ours */ }
    }
  }
  function onStageAsks(e) {
    if (e.data?.type !== STAGE_MSG || e.data.event !== 'which') return;
    if (e.origin !== window.location.origin || !e.source || e.source.parent !== window) return;
    e.source.postMessage({ type: STAGE_MSG, session: sessionId }, e.origin);
  }
  window.addEventListener('message', onStageAsks);

  function handleMessage(e) {
    if (e.data?.type !== 'linggen-skill-event') return;
    if (!iframe || e.source !== iframe.contentWindow || e.origin !== chatOrigin) return;
    if (!embedAlive) flushOutbox();
    const { event, payload } = e.data;
    const handler = handlers[event];
    // Anything else the engine relays (a save changed under the page, …)
    // goes to the page as it came.
    if (handler) handler(payload);
    else options.onSkillEvent?.(event, payload);
  }
  window.addEventListener('message', handleMessage);

  function loaded() {
    return new Promise((resolve) => iframe.addEventListener('load', resolve, { once: true }));
  }

  async function ensureMounted() {
    if (mounting) return mounting;
    mounting = (async () => {
      if (!sessionId) {
        const data = await createSession(options.title || `${skillName} session`, skillName);
        sessionId = data.id;
        tellStages();
        if (onSessionCreated) onSessionCreated(sessionId);
      }
      iframe = document.createElement('iframe');
      iframe.src = buildSrc(sessionId);
      iframe.style.cssText = 'width:100%;height:100%;border:none;';
      iframe.allow = 'clipboard-write';
      if (localPanel) { localPanel.remove(); localPanel = null; }
      el.appendChild(iframe);
      await loaded();
      // Replay the local welcome ahead of anything queued, so the swap reads
      // as one conversation.
      outbox.unshift(...localLog.map((m) => ({ type: 'linggen-skill', action: 'add_message', payload: m })));
      armGracePeriod();
    })();
    return mounting;
  }

  if (!lazy) await ensureMounted();
  tellStages();

  // A page whose sessions are one-offs (a match, a draft) deletes its session
  // on leave — otherwise every visit would leave one behind.
  function onPageHide() {
    if (sessionId) removeSkillSession(skillName, sessionId, { keepalive: true }).catch(() => {});
  }
  if (options.deleteOnLeave) window.addEventListener('pagehide', onPageHide);

  /** Send a message to the chat (the embed submits it). */
  function send(text) {
    streamBuffers.clear();
    ensureMounted();
    post({ type: 'linggen-skill', action: 'send', payload: { text } });
  }

  /** Send a message to the agent without showing it in the chat. */
  function sendHidden(text) {
    streamBuffers.clear();
    ensureMounted();
    post({ type: 'linggen-skill', action: 'send_hidden', payload: { text } });
  }

  /** Add a local-only message to the chat display. */
  function addMessage(role, text) {
    const mappedRole = (role === 'ai' || role === 'assistant') ? 'assistant' : role;
    if (!iframe) {
      localLog.push({ role: mappedRole, text });
      renderLocalMessage(mappedRole, text);
      return;
    }
    post({ type: 'linggen-skill', action: 'add_message', payload: { role: mappedRole, text } });
  }

  /** The buttons above the chat for what the page shows; [] hands the row back to the skill's starters. */
  function setSuggestions(items) {
    post({ type: 'linggen-skill', action: 'set_suggestions', payload: { items: Array.isArray(items) ? items : [] } });
  }

  return {
    send,
    sendHidden,
    addMessage,
    setSuggestions,
    destroy() {
      window.removeEventListener('message', handleMessage);
      window.removeEventListener('message', onStageAsks);
      sessionId = null;
      tellStages();
      window.removeEventListener('pagehide', onPageHide);
      if (localPanel) { localPanel.remove(); localPanel = null; }
      if (iframe) iframe.remove();
    },
    async deleteSession() {
      if (!sessionId) return;
      try { await removeSkillSession(skillName, sessionId); } catch { /* ignore */ }
      sessionId = null;
      tellStages();
    },
    getSessionId() { return sessionId; },
    setOptions(opts) {
      if (opts.modelId) modelId = opts.modelId;
    },
    /** Re-point the chat at another session in place: only this iframe
     *  reloads, the host page stays mounted. Resolves once it has loaded. */
    async setSession(sid) {
      if (!sid || sid === sessionId) return;
      sessionId = sid;
      tellStages();
      streamBuffers.clear();
      if (!iframe) return; // lazy and not yet mounted: the first send uses it
      embedAlive = false;
      const done = loaded();
      iframe.src = buildSrc(sid);
      await done;
      armGracePeriod();
    },
    /** 'connected' | 'reconnecting' | 'disconnected' — the iframe's transport. */
    connectionStatus: () => connectionStatus,
  };
}

window.LinggenUI = { mount };

// ── Presence beat ───────────────────────────────────────────────────────────
// Somebody reading an app page is present. Same beat as the main UI's
// presence.ts — recency, focus and a typing flag, never a keystroke — plus
// which app is in front. The engine keeps the most present of the live
// surfaces, so a blurred tab beside this one cannot erase it.
(function () {
  const BEAT_MS = 4000;
  const TYPING_WINDOW_MS = 1500;
  // Remotely the page lives at /tunnel/<instance>/apps/<skill>/… (linggen.dev).
  const APP = (location.pathname.match(/^(?:\/tunnel\/[^/]+)?\/apps\/([a-z0-9-]+)\//) || [])[1] || null;
  let lastInputAt = Date.now();
  let lastKeyAt = 0;

  const beat = () => {
    const now = Date.now();
    // Focus is what makes it Linggen the person is in: keystrokes in another
    // window never reach this page, so a blurred or hidden tab is away.
    const focused = document.hasFocus() && document.visibilityState === 'visible';
    fetch('/api/presence', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        ...(APP ? { app: APP } : {}),
        focused,
        typing: focused && now - lastKeyAt < TYPING_WINDOW_MS,
        idle_ms: now - lastInputAt,
      }),
      keepalive: true,
    }).catch(() => { /* offline, or no daemon — nothing to do about it here */ });
  };

  window.addEventListener('keydown', () => { lastInputAt = lastKeyAt = Date.now(); }, { passive: true });
  window.addEventListener('pointermove', () => { lastInputAt = Date.now(); }, { passive: true });
  window.addEventListener('focus', beat);
  window.addEventListener('blur', beat);
  document.addEventListener('visibilitychange', beat);

  beat();
  setInterval(beat, BEAT_MS);
})();
