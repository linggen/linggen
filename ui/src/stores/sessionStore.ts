/**
 * Session and workspace selection state.
 */
import { create } from 'zustand';
import type { AskUserQuestion, SessionInfo } from '../types';
import { sessions as sessionsApi } from '../lib/api';
import { confirmDialog } from '../lib/confirmDialog';
import { useInteractionStore } from './interactionStore';
import { sessionApi } from '../lib/endpoints';
import { newChatRequest } from '../lib/sessionBinding.mts';
import { afterNewChat } from '../lib/sessionRoot.mts';

const ACTIVE_SESSION_STORAGE_KEY = 'linggen:active-session';

interface SessionState {
  /** The open session's folder (`lib/sessionRoot.mts`): in memory only, so
   *  a folder picked for one session never reaches the next New chat. */
  selectedProjectRoot: string;
  sessions: SessionInfo[];
  /** Unified list of ALL sessions across projects, missions, skills. */
  allSessions: SessionInfo[];
  /** The engine has pushed `allSessions` once — until then the list is loading, not empty. */
  allSessionsLoaded: boolean;
  activeSessionId: string | null;
  isMissionSession: boolean;
  activeMissionId: string | null;
  isSkillSession: boolean;
  activeSkillName: string | null;
  /** An app page's chat: the values its sessions are bound to (the page's
   *  `mount(el, {params})`) — every new chat through it carries them. */
  boundParams: Record<string, string>;

  // Actions
  setSelectedProjectRoot: (root: string) => void;
  setActiveSessionId: (id: string | null) => void;
  setIsMissionSession: (val: boolean) => void;
  setActiveMissionId: (id: string | null) => void;
  setIsSkillSession: (val: boolean) => void;
  setActiveSkillName: (name: string | null) => void;

  // No-ops — data arrives via page_state push. Kept for call-site compat.
  fetchSessions: () => Promise<void>;
  fetchAllSessions: () => Promise<void>;

  createSession: () => Promise<void>;
  /** A New chat being made: a message sent meanwhile waits for it — it
   *  belongs to the new chat, not the one on screen before the click. */
  newChat: Promise<string | null> | null;
  removeSession: (id: string) => Promise<void>;
  removeSessions: (ids: string[]) => Promise<void>;
  renameSession: (id: string, title: string) => Promise<void>;
}

export const useSessionStore = create<SessionState>((set, get) => ({
  selectedProjectRoot: '',
  sessions: [],
  allSessions: [],
  allSessionsLoaded: false,
  // Deep link: `/?session=<id>` opens that session directly — skill pages
  // link into mission runs this way (the memory calendar's "dreaming in
  // mission" chip). Compact/embed mode has always relied on this param;
  // with no param the page-state push auto-picks (last used, else first).
  activeSessionId: (() => {
    if (typeof window === 'undefined') return null;
    return new URLSearchParams(window.location.search).get('session');
  })(),
  isMissionSession: false,
  activeMissionId: null,
  isSkillSession: false,
  activeSkillName: null,
  boundParams: {},

  setSelectedProjectRoot: (root) => set({ selectedProjectRoot: root }),
  setActiveSessionId: (id) => {
    if (id) window.localStorage.setItem(ACTIVE_SESSION_STORAGE_KEY, id);
    else window.localStorage.removeItem(ACTIVE_SESSION_STORAGE_KEY);
    set({ activeSessionId: id });
    void refreshPendingAskFor(id);
  },
  setIsMissionSession: (val) => set({ isMissionSession: val }),
  setActiveMissionId: (id) => set({ activeMissionId: id }),
  setIsSkillSession: (val) => set({ isSkillSession: val }),
  setActiveSkillName: (name) => set({ activeSkillName: name }),

  // Data arrives via page_state push — no HTTP fetch needed
  fetchSessions: async () => {},
  fetchAllSessions: async () => {},

  newChat: null,

  createSession: async () => {
    const making = makeSession(set);
    set({ newChat: making });
    await making;
    if (get().newChat === making) set({ newChat: null });
  },

  removeSession: async (id) => {
    const { allSessions } = get();
    if (!(await confirmDialog('Remove this session?'))) return;
    const session = allSessions.find(s => s.id === id);
    try {
      await sessionsApi.remove({
        session_id: id,
        project: session?.project || null,
        mission_id: session?.mission_id || null,
        skill: session?.skill || null,
      });
      // Optimistically remove from local state (no refetch needed — page_state will confirm)
      set((s) => {
        const newAll = s.allSessions.filter(sess => sess.id !== id);
        const newSessions = s.sessions.filter(sess => sess.id !== id);
        const patch: Partial<SessionState> = { allSessions: newAll, sessions: newSessions };
        if (s.activeSessionId === id) {
          patch.activeSessionId = newAll.length > 0 ? newAll[0].id : null;
          if (patch.activeSessionId) {
            window.localStorage.setItem(ACTIVE_SESSION_STORAGE_KEY, patch.activeSessionId);
          } else {
            window.localStorage.removeItem(ACTIVE_SESSION_STORAGE_KEY);
          }
        }
        return patch;
      });
    } catch (e) {
      console.error('Error removing session:', e);
    }
  },

  removeSessions: async (ids) => {
    if (ids.length === 0) return;
    const { allSessions } = get();
    const msg = ids.length === 1 ? 'Remove this session?' : `Remove ${ids.length} sessions?`;
    if (!(await confirmDialog(msg))) return;
    const idSet = new Set(ids);
    const targets = allSessions.filter(s => idSet.has(s.id));
    await Promise.allSettled(targets.map(session =>
      sessionsApi.remove({
        session_id: session.id,
        project: session.project || null,
        mission_id: session.mission_id || null,
        skill: session.skill || null,
      }),
    ));
    set((s) => {
      const newAll = s.allSessions.filter(sess => !idSet.has(sess.id));
      const newSessions = s.sessions.filter(sess => !idSet.has(sess.id));
      const patch: Partial<SessionState> = { allSessions: newAll, sessions: newSessions };
      if (s.activeSessionId && idSet.has(s.activeSessionId)) {
        patch.activeSessionId = newAll.length > 0 ? newAll[0].id : null;
        if (patch.activeSessionId) {
          window.localStorage.setItem(ACTIVE_SESSION_STORAGE_KEY, patch.activeSessionId);
        } else {
          window.localStorage.removeItem(ACTIVE_SESSION_STORAGE_KEY);
        }
      }
      return patch;
    });
  },

  // Sessions are global: the id names one, no project root is needed.
  renameSession: async (id, title) => {
    try {
      await sessionsApi.rename({ session_id: id, title });
      set((s) => ({
        allSessions: s.allSessions.map(sess => sess.id === id ? { ...sess, title } : sess),
        sessions: s.sessions.map(sess => sess.id === id ? { ...sess, title } : sess),
      }));
    } catch (e) {
      console.error('Error renaming session:', e);
    }
  },

}));

/**
 * The project root a call about the open session works in: `override` >
 * the open session's folder as mirrored (`selectedProjectRoot`) > its own
 * project/cwd, for a session opened by a link before the mirror is set.
 * The engine prefers the session's own folder over any root sent.
 */
export function effectiveProjectRoot(override?: string | null): string {
  if (override) return override;
  const state = useSessionStore.getState();
  if (state.selectedProjectRoot) return state.selectedProjectRoot;
  const sess = state.allSessions.find((s) => s.id === state.activeSessionId);
  return sess?.project || sess?.cwd || '';
}

/** Makes a new chat session and opens it; its id, or null when it failed. */
async function makeSession(set: (fn: (s: SessionState) => Partial<SessionState>) => void): Promise<string | null> {
  const now = new Date();
  const title = `Chat ${now.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}, ${now.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })}`;
  try {
    // Never a folder: the engine starts a New chat in the default workspace.
    const data = await sessionsApi.create(newChatRequest(title));
    set((s) => afterNewChat(s.allSessions, { id: data.id, title, cwd: data.cwd }, Math.floor(Date.now() / 1000)));
    window.localStorage.setItem(ACTIVE_SESSION_STORAGE_KEY, data.id);
    return data.id;
  } catch (e) {
    console.error('Error creating session:', e);
    return null;
  }
}

// An AskUser raised while its session wasn't on screen is dropped by the
// live event filter (eventHandlers/interactive.ts) — so entering a session
// must pull the server's pending list, or the ask stays invisible until it
// times out (observed on an attended dream-mission run, 2026-08-18). The
// stale slot from the previous session is cleared either way.
async function refreshPendingAskFor(sessionId: string | null): Promise<void> {
  useInteractionStore.getState().setPendingAskUser(null);
  if (!sessionId) return;
  try {
    const items = await sessionApi.pendingAskUser();
    const mine = items.find((it) => !it.session_id || it.session_id === sessionId);
    // The user may have switched again while the fetch ran.
    if (useSessionStore.getState().activeSessionId !== sessionId) return;
    if (!mine) return;
    useInteractionStore.getState().setPendingAskUser({
      questionId: mine.question_id,
      agentId: String(mine.agent_id || ''),
      questions: (mine.questions || []) as AskUserQuestion[],
    });
  } catch {
    // Daemon unreachable — the live event path still covers new asks.
  }
}
