/**
 * Session and workspace selection state.
 */
import { create } from 'zustand';
import type { SessionInfo } from '../types';

const SELECTED_PROJECT_STORAGE_KEY = 'linggen:selected-project';
const ACTIVE_SESSION_STORAGE_KEY = 'linggen:active-session';

interface SessionState {
  selectedProjectRoot: string;
  sessions: SessionInfo[];
  /** Unified list of ALL sessions across projects, missions, skills. */
  allSessions: SessionInfo[];
  activeSessionId: string | null;
  isMissionSession: boolean;
  activeMissionId: string | null;
  isSkillSession: boolean;
  activeSkillName: string | null;

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
  removeSession: (id: string) => Promise<void>;
  renameSession: (id: string, title: string) => Promise<void>;
}

export const useSessionStore = create<SessionState>((set, get) => ({
  selectedProjectRoot: typeof window !== 'undefined'
    ? window.localStorage.getItem(SELECTED_PROJECT_STORAGE_KEY) || ''
    : '',
  sessions: [],
  allSessions: [],
  // Session ID is validated against the fetched sessions list in fetchSessions().
  // Only compact/embed mode sets it immediately (from URL params).
  activeSessionId: (() => {
    if (typeof window === 'undefined') return null;
    const params = new URLSearchParams(window.location.search);
    if (params.get('mode') === 'compact' && params.get('session')) {
      return params.get('session');
    }
    return null;
  })(),
  isMissionSession: false,
  activeMissionId: null,
  isSkillSession: false,
  activeSkillName: null,

  setSelectedProjectRoot: (root) => {
    if (root) window.localStorage.setItem(SELECTED_PROJECT_STORAGE_KEY, root);
    else window.localStorage.removeItem(SELECTED_PROJECT_STORAGE_KEY);
    set({ selectedProjectRoot: root });
  },
  setActiveSessionId: (id) => {
    if (id) window.localStorage.setItem(ACTIVE_SESSION_STORAGE_KEY, id);
    else window.localStorage.removeItem(ACTIVE_SESSION_STORAGE_KEY);
    set({ activeSessionId: id });
  },
  setIsMissionSession: (val) => set({ isMissionSession: val }),
  setActiveMissionId: (id) => set({ activeMissionId: id }),
  setIsSkillSession: (val) => set({ isSkillSession: val }),
  setActiveSkillName: (name) => set({ activeSkillName: name }),

  // Data arrives via page_state push — no HTTP fetch needed
  fetchSessions: async () => {},
  fetchAllSessions: async () => {},

  createSession: async () => {
    const now = new Date();
    const title = `Chat ${now.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}, ${now.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })}`;
    try {
      const resp = await fetch('/api/sessions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ title }),
      });
      const data = await resp.json();
      // Optimistically add to local state
      const newSession = { id: data.id, repo_path: '', title, created_at: Math.floor(Date.now() / 1000) };
      set((s) => ({
        activeSessionId: data.id,
        allSessions: [newSession, ...s.allSessions],
      }));
      window.localStorage.setItem(ACTIVE_SESSION_STORAGE_KEY, data.id);
    } catch (e) {
      console.error('Error creating session:', e);
    }
  },

  removeSession: async (id) => {
    const { allSessions } = get();
    if (!confirm('Remove this session?')) return;
    // Find session metadata to route the delete to the correct store
    const session = allSessions.find(s => s.id === id);
    try {
      await fetch('/api/sessions/all', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          session_id: id,
          project: session?.project || null,
          mission_id: session?.mission_id || null,
          skill: session?.skill || null,
        }),
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

  renameSession: async (id, title) => {
    const { selectedProjectRoot } = get();
    if (!selectedProjectRoot) return;
    try {
      await fetch('/api/sessions', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot, session_id: id, title }),
      });
      // Optimistically update title in local state
      set((s) => ({
        allSessions: s.allSessions.map(sess => sess.id === id ? { ...sess, title } : sess),
        sessions: s.sessions.map(sess => sess.id === id ? { ...sess, title } : sess),
      }));
    } catch (e) {
      console.error('Error renaming session:', e);
    }
  },

}));
