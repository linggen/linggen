/**
 * Embed entry — chat-only surface for skill iframes and VS Code extension.
 * Pinned to a single session; no header, sidebar, settings, or missions.
 *
 * URL params:
 *   session  — pinned session id (skill iframe path)
 *   skill    — bound skill name (marks the session as skill-session)
 *   project  — VS Code project root (auto-creates/resumes a "VS Code" session)
 *   hide_toolbar — 1 to hide the compact toolbar (when parent provides its own chrome)
 */
import React, { useCallback, useEffect, useRef } from 'react';
import { Plus } from 'lucide-react';
import { ChatWidget } from '../components/chat';
import { ToastContainer } from '../components/ToastContainer';
import { useSessionStore } from '../stores/sessionStore';
import { useServerStore } from '../stores/serverStore';
import { useChatStore } from '../stores/chatStore';
import { useUiStore } from '../stores/uiStore';
import { useUserStore } from '../stores/userStore';
import { useChatActions } from '../hooks/useChatActions';
import { useRunInfo } from '../hooks/useRunInfo';

const params = new URLSearchParams(window.location.search);
const pinnedSession = params.get('session') || '';
const pinnedSkill = params.get('skill') || '';
const pinnedModel = params.get('model') || '';
const vscProject = params.get('project') || '';
const hideToolbar = params.get('hide_toolbar') === '1';

/** Detect remote/tunnel mode (blob iframe with injected instance meta tag). */
const isRemoteMode = typeof document !== 'undefined' && !!document.querySelector('meta[name="linggen-instance"]');

export const EmbedApp: React.FC = () => {
  const projectStore = useSessionStore();
  const agentStore = useServerStore();

  const { sessions, activeSessionId } = projectStore;
  const { agents, selectedAgent } = agentStore;
  const isRunning = agentStore.isRunning();

  // --- Run info + chat actions (for clipboard bridge) ---
  const { runningMainRunIds } = useRunInfo();
  const scrollToBottomNoop = useCallback(() => {}, []);
  const { clearChat, sendChatMessage } = useChatActions(scrollToBottomNoop, runningMainRunIds);

  // --- Init: pin session or auto-create VS Code session ---
  const initRef = useRef(false);
  useEffect(() => {
    if (initRef.current) return;
    initRef.current = true;

    // Skill-bound iframe: session created by SDK wrapper.
    // Clear selectedProjectRoot in memory so API calls use the session's cwd.
    if (pinnedSession) {
      useSessionStore.setState({
        activeSessionId: pinnedSession,
        selectedProjectRoot: '',
        ...(pinnedSkill ? { isSkillSession: true, activeSkillName: pinnedSkill } : {}),
      });
      // Honor the model the host skill pinned (?model=…). useChatActions reads
      // uiStore.sessionModel at send time; without this the run silently falls
      // back to the engine default and the selector shows "Default (…)".
      if (pinnedModel) useUiStore.getState().setSessionModel(pinnedModel);
      const cs = useChatStore.getState();
      cs.setActiveSession(pinnedSession);
      cs.fetchSessionState();
      return;
    }

    // VS Code compact mode: auto-create/resume project session.
    if (!vscProject) return;
    useSessionStore.getState().setSelectedProjectRoot(vscProject);
    (async () => {
      try {
        const resp = await fetch(`/api/sessions?project_root=${encodeURIComponent(vscProject)}`);
        const data = await resp.json();
        const sessionList = data.sessions ?? data ?? [];
        const existing = sessionList.find((s: any) => s.title?.startsWith('VS Code'));
        if (existing) {
          useSessionStore.getState().setActiveSessionId(existing.id);
        } else {
          const createResp = await fetch('/api/sessions', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ project_root: vscProject, title: 'VS Code' }),
          });
          const created = await createResp.json();
          useSessionStore.getState().setActiveSessionId(created.id);
        }
      } catch (e) {
        console.error('Embed session init error:', e);
      }
    })();
  }, []);

  // --- Clipboard + skill command bridge (postMessage from parent page) ---
  const clearChatRef = useRef(clearChat);
  const sendChatMessageRef = useRef(sendChatMessage);
  useEffect(() => { clearChatRef.current = clearChat; sendChatMessageRef.current = sendChatMessage; }, [clearChat, sendChatMessage]);

  useEffect(() => {
    // Apply VS Code dark theme only outside skill iframes (skill pages follow system scheme).
    const applyCompactTheme = !pinnedSkill;
    if (applyCompactTheme) {
      document.documentElement.setAttribute('data-compact', '');
    }
    const handleMessage = (e: MessageEvent) => {
      if (e.data?.type !== 'linggen-clipboard') return;
      const sel = window.getSelection();
      const text = sel?.toString();
      switch (e.data.action) {
        case 'copy': if (text) navigator.clipboard.writeText(text).catch(() => {}); break;
        case 'cut': if (text) { navigator.clipboard.writeText(text).catch(() => {}); document.execCommand('delete'); } break;
        case 'selectAll': document.execCommand('selectAll'); break;
      }
    };
    const handleCopy = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 'c' && !e.shiftKey && !e.altKey) {
        const sel = window.getSelection();
        const text = sel?.toString();
        if (text) { e.preventDefault(); navigator.clipboard.writeText(text).catch(() => {}); }
      }
    };
    const handleSkillCommand = (e: MessageEvent) => {
      if (e.data?.type !== 'linggen-skill') return;
      const { action, payload } = e.data;
      switch (action) {
        case 'send': {
          sendChatMessageRef.current(payload?.text || '');
          break;
        }
        case 'send_hidden': {
          const hiddenText = payload?.text || '';
          if (hiddenText) sendChatMessageRef.current(`[HIDDEN] ${hiddenText}`);
          break;
        }
        case 'add_message':
          useChatStore.getState().addMessage({
            role: payload?.role === 'user' ? 'user' as const : 'agent' as const,
            from: payload?.role === 'user' ? 'user' : 'assistant',
            to: '',
            text: payload?.text || '',
            timestamp: new Date().toLocaleTimeString(),
            timestampMs: Date.now(),
            isGenerating: false,
          });
          break;
        case 'clear':
          clearChatRef.current();
          break;
      }
    };
    window.addEventListener('message', handleSkillCommand);
    window.addEventListener('message', handleMessage);
    document.addEventListener('keydown', handleCopy);
    // Tell the parent skill page we can now receive posted messages — its
    // chat-bridge queues sends until it sees life from this iframe.
    try { window.parent.postMessage({ type: 'linggen-skill-event', event: 'ready', payload: {} }, '*'); } catch { /* not embedded */ }
    return () => {
      window.removeEventListener('message', handleSkillCommand);
      window.removeEventListener('message', handleMessage);
      document.removeEventListener('keydown', handleCopy);
      if (applyCompactTheme) {
        document.documentElement.removeAttribute('data-compact');
      }
    };
  }, []);

  // --- Remote mode: gate rendering until transport is connected ---
  const connectionStatus = useUserStore((s) => s.connectionStatus);
  if (isRemoteMode && connectionStatus === 'disconnected') {
    return (
      <div className="h-screen flex items-center justify-center bg-white dark:bg-[#0b0e14]">
        <div className="text-center space-y-4">
          <div className="inline-block animate-spin rounded-full h-10 w-10 border-4 border-slate-200 dark:border-slate-700 border-t-emerald-500" />
          <p className="text-slate-500 dark:text-slate-400 text-sm">Connecting to linggen server…</p>
        </div>
      </div>
    );
  }

  return (
    <>
      <ToastContainer />
      <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden">
        {!hideToolbar && (
          <div className="flex items-center gap-1.5 px-2 py-1 border-b border-slate-200 dark:border-white/10 bg-white dark:bg-[#0f0f0f] flex-shrink-0">
            <select
              value={selectedAgent}
              onChange={(e) => agentStore.setSelectedAgent(e.target.value)}
              className="text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 text-slate-700 dark:text-slate-300 outline-none max-w-[5rem]"
            >
              {agents.map((a) => <option key={a.name} value={a.name}>{a.name}</option>)}
            </select>
            <select
              value={activeSessionId || ''}
              onChange={(e) => { projectStore.setActiveSessionId(e.target.value || null); projectStore.setIsMissionSession(false); }}
              className="text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 text-slate-700 dark:text-slate-300 outline-none flex-1 min-w-0 truncate"
            >
              {sessions.length === 0 && <option value="">No sessions</option>}
              {sessions.map((s) => <option key={s.id} value={s.id}>{s.title || s.id.slice(0, 8)}</option>)}
            </select>
            <button
              onClick={() => projectStore.createSession()}
              title="New chat session"
              className="p-0.5 rounded hover:bg-slate-200 dark:hover:bg-white/10 text-slate-500 hover:text-slate-700 dark:hover:text-slate-300 transition-colors flex-shrink-0"
            >
              <Plus size={14} />
            </button>
            <span className={`text-[11px] flex-shrink-0 ${isRunning ? 'text-green-500' : 'text-slate-400'}`}>
              {isRunning ? 'Running' : 'Idle'}
            </span>
          </div>
        )}

        <div className="flex-1 min-h-0">
          <ChatWidget
            sessionId={activeSessionId}
            projectRoot={useSessionStore.getState().selectedProjectRoot}
            mode="compact"
          />
        </div>
      </div>

      <style>{`
        html[data-compact] { --vsc-bg: #1e1e1e; --vsc-sidebar: #252526; --vsc-input: #3c3c3c; --vsc-border: #3c3c3c; --vsc-fg: #cccccc; --vsc-fg-muted: #858585; --vsc-accent: #0e639c; color-scheme: dark; }
        html[data-compact] .dark\\:bg-\\[\\#0a0a0a\\] { background-color: var(--vsc-bg) !important; }
        html[data-compact] .dark\\:bg-\\[\\#0f0f0f\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:bg-white\\/\\[0\\.02\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:bg-white\\/5 { background-color: rgba(255,255,255,0.03) !important; }
        html[data-compact] .dark\\:bg-black\\/20 { background-color: var(--vsc-input) !important; }
        html[data-compact] .dark\\:bg-black\\/30 { background-color: var(--vsc-input) !important; }
        html[data-compact] .dark\\:border-white\\/5, html[data-compact] .dark\\:border-white\\/10 { border-color: var(--vsc-border) !important; }
        html[data-compact] section { border-radius: 0 !important; border: none !important; }
        html[data-compact] .dark\\:text-slate-200 { color: var(--vsc-fg) !important; }
        html[data-compact] .dark\\:text-slate-300 { color: #d4d4d4 !important; }
        html[data-compact] .dark\\:text-slate-400 { color: #969696 !important; }
        html[data-compact] .text-slate-500 { color: #969696 !important; }
        html[data-compact] .text-slate-400 { color: #969696 !important; }
        html[data-compact] .text-slate-600 { color: #b0b0b0 !important; }
        html[data-compact] select, html[data-compact] input, html[data-compact] textarea { color: var(--vsc-fg) !important; background-color: var(--vsc-input) !important; border-color: var(--vsc-border) !important; font-size: 13px !important; }
        html[data-compact] ::placeholder { color: var(--vsc-fg-muted) !important; opacity: 1 !important; }
        html[data-compact] body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; font-size: 13px; color: var(--vsc-fg); }
        html[data-compact] .custom-scrollbar::-webkit-scrollbar-thumb { background: rgba(121,121,121,0.4); border-radius: 0; }
        html[data-compact] .custom-scrollbar::-webkit-scrollbar-thumb:hover { background: rgba(121,121,121,0.7); }
        html[data-compact] .custom-scrollbar::-webkit-scrollbar-track { background: transparent; }
        html[data-compact] .dark\\:bg-\\[\\#141414\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:border-amber-500\\/20, html[data-compact] .dark\\:border-amber-500\\/10 { border-color: var(--vsc-border) !important; }
        html[data-compact] .dark\\:bg-amber-500\\/5 { background-color: rgba(255,255,255,0.03) !important; }
        html[data-compact] .dark\\:bg-amber-500\\/10 { background-color: rgba(255,255,255,0.05) !important; }
        html[data-compact] .dark\\:text-amber-400 { color: #cca700 !important; }
        html[data-compact] .dark\\:text-amber-300 { color: #ddb700 !important; }
        html[data-compact] .dark\\:text-amber-300\\/80 { color: rgba(221,183,0,0.8) !important; }
        html[data-compact] .dark\\:hover\\:border-amber-500\\/40:hover { border-color: #cca700 !important; }
        html[data-compact] .dark\\:hover\\:bg-amber-500\\/5:hover { background-color: rgba(255,255,255,0.05) !important; }
        html[data-compact] .dark\\:border-amber-500\\/30 { border-color: rgba(204,167,0,0.4) !important; }
        html[data-compact] .dark\\:border-blue-500\\/20, html[data-compact] .dark\\:border-blue-500\\/10 { border-color: var(--vsc-border) !important; }
        html[data-compact] .dark\\:bg-blue-500\\/5 { background-color: rgba(255,255,255,0.03) !important; }
        html[data-compact] .dark\\:text-blue-400 { color: #569cd6 !important; }
        html[data-compact] .dark\\:hover\\:border-blue-500\\/40:hover { border-color: #569cd6 !important; }
        html[data-compact] .dark\\:bg-blue-500 { background-color: var(--vsc-accent) !important; border-color: var(--vsc-accent) !important; }
        html[data-compact] .dark\\:hover\\:bg-blue-600:hover { background-color: #1177bb !important; }
        html[data-compact] .dark\\:bg-white\\/5 { background-color: var(--vsc-input) !important; border-color: var(--vsc-border) !important; }
        html[data-compact] .dark\\:hover\\:bg-white\\/10:hover { background-color: rgba(255,255,255,0.08) !important; }
        html[data-compact] .dark\\:hover\\:border-red-500\\/40:hover { border-color: #f14c4c !important; }
        html[data-compact] .dark\\:hover\\:text-red-400:hover { color: #f14c4c !important; }
        html[data-compact] .dark\\:hover\\:border-slate-400\\/30:hover { border-color: var(--vsc-fg-muted) !important; }
        html[data-compact] .dark\\:hover\\:text-slate-400:hover { color: #b0b0b0 !important; }
        html[data-compact] .dark\\:hover\\:text-slate-300:hover { color: #d4d4d4 !important; }
        html[data-compact] .dark\\:hover\\:text-amber-400:hover { color: #cca700 !important; }
        html[data-compact] .dark\\:hover\\:border-amber-500\\/30:hover { border-color: rgba(204,167,0,0.4) !important; }
        html[data-compact] .dark\\:bg-amber-600 { background-color: #b8860b !important; }
        html[data-compact] .dark\\:hover\\:bg-amber-700:hover { background-color: #996f0a !important; }
        html[data-compact] section { border-radius: 2px !important; border: 1px solid var(--vsc-border) !important; }
        html[data-compact] .rounded-xl { border-radius: 2px !important; }
        html[data-compact] .rounded-lg { border-radius: 2px !important; }
        html[data-compact] .rounded-md { border-radius: 2px !important; }
        html[data-compact] .shadow-sm { box-shadow: none !important; }
        html[data-compact] .shadow-xl { box-shadow: 0 2px 8px rgba(0,0,0,0.36) !important; }
        html[data-compact] .dark\\:text-green-300 { color: #89d185 !important; }
        html[data-compact] .dark\\:text-blue-300 { color: #569cd6 !important; }
        html[data-compact] .dark\\:text-amber-300 { color: #cca700 !important; }
        html[data-compact] .dark\\:text-indigo-300 { color: #b4a0ff !important; }
        html[data-compact] .dark\\:text-slate-600 { color: #5a5a5a !important; }
        html[data-compact] .dark\\:text-slate-500 { color: #6e6e6e !important; }
        html[data-compact] .dark\\:focus\\:border-amber-500\\/40:focus { border-color: var(--vsc-accent) !important; }
        html[data-compact] .dark\\:focus\\:border-blue-500\\/40:focus { border-color: var(--vsc-accent) !important; }
        html[data-compact] .dark\\:bg-white\\/\\[0\\.02\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:hover\\:bg-white\\/5:hover { background-color: rgba(255,255,255,0.06) !important; }
        html[data-compact] .markdown-body { color: var(--vsc-fg, #ccc) !important; }
        html[data-compact] .markdown-body code { background: rgba(255,255,255,0.1) !important; color: #d4d4d4 !important; }
        html[data-compact] .markdown-body pre { background: rgba(255,255,255,0.06) !important; }
        html[data-compact] .markdown-body pre code { color: #c9d1d9 !important; }
        html[data-compact] .markdown-body th, html[data-compact] .markdown-body td { border-color: rgba(255,255,255,0.15) !important; color: #d4d4d4 !important; }
        html[data-compact] .markdown-body th { background: rgba(255,255,255,0.06) !important; }
        html[data-compact] .markdown-body hr { border-top-color: rgba(255,255,255,0.12) !important; }
        html[data-compact] .markdown-body blockquote { border-left-color: rgba(59,130,246,0.5) !important; color: #b4becd !important; }
        html[data-compact] .markdown-body h1, html[data-compact] .markdown-body h2, html[data-compact] .markdown-body h3, html[data-compact] .markdown-body h4 { color: #e0e0e0 !important; }
        html[data-compact] .hljs { color: #c9d1d9 !important; background: transparent !important; }
        html[data-compact] .hljs-doctag, html[data-compact] .hljs-keyword, html[data-compact] .hljs-template-tag, html[data-compact] .hljs-template-variable, html[data-compact] .hljs-type, html[data-compact] .hljs-variable.language_ { color: #ff7b72 !important; }
        html[data-compact] .hljs-title, html[data-compact] .hljs-title.class_, html[data-compact] .hljs-title.function_ { color: #d2a8ff !important; }
        html[data-compact] .hljs-attr, html[data-compact] .hljs-attribute, html[data-compact] .hljs-literal, html[data-compact] .hljs-meta, html[data-compact] .hljs-number, html[data-compact] .hljs-operator, html[data-compact] .hljs-variable, html[data-compact] .hljs-selector-attr, html[data-compact] .hljs-selector-class, html[data-compact] .hljs-selector-id { color: #79c0ff !important; }
        html[data-compact] .hljs-regexp, html[data-compact] .hljs-string { color: #a5d6ff !important; }
        html[data-compact] .hljs-built_in, html[data-compact] .hljs-symbol { color: #ffa657 !important; }
        html[data-compact] .hljs-comment, html[data-compact] .hljs-code, html[data-compact] .hljs-formula { color: #8b949e !important; }
        html[data-compact] .hljs-name, html[data-compact] .hljs-quote, html[data-compact] .hljs-selector-tag, html[data-compact] .hljs-selector-pseudo { color: #7ee787 !important; }
        html[data-compact] .hljs-subst { color: #c9d1d9 !important; }
        html[data-compact] .hljs-addition { color: #aff5b4 !important; background-color: #033a16 !important; }
        html[data-compact] .hljs-deletion { color: #ffdcd7 !important; background-color: #67060c !important; }
      `}</style>
    </>
  );
};
