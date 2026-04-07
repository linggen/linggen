/**
 * Project, session, and workspace tree state.
 */
import { create } from 'zustand';
import type { ProjectInfo, SessionInfo, AgentTreeItem } from '../types';
import { dedupFetch } from '../lib/dedupFetch';

const SELECTED_PROJECT_STORAGE_KEY = 'linggen:selected-project';
const ACTIVE_SESSION_STORAGE_KEY = 'linggen:active-session';

interface ProjectState {
  projects: ProjectInfo[];
  selectedProjectRoot: string;
  sessions: SessionInfo[];
  /** Unified list of ALL sessions across projects, missions, skills. */
  allSessions: SessionInfo[];
  activeSessionId: string | null;
  isMissionSession: boolean;
  activeMissionId: string | null;
  activeMissionProject: string | null;
  isSkillSession: boolean;
  activeSkillName: string | null;
  agentTreesByProject: Record<string, Record<string, AgentTreeItem>>;
  currentPath: string;

  // Add-project UI
  newProjectPath: string;
  showAddProject: boolean;

  // Actions
  setSelectedProjectRoot: (root: string) => void;
  setActiveSessionId: (id: string | null) => void;
  setIsMissionSession: (val: boolean) => void;
  setActiveMissionId: (id: string | null) => void;
  setActiveMissionProject: (project: string | null) => void;
  setIsSkillSession: (val: boolean) => void;
  setActiveSkillName: (name: string | null) => void;
  setNewProjectPath: (path: string) => void;
  setShowAddProject: (show: boolean) => void;
  setCurrentPath: (path: string) => void;

  fetchProjects: () => Promise<void>;
  addProject: () => Promise<void>;
  removeProject: (path: string) => Promise<void>;
  pickFolder: () => Promise<void>;

  fetchSessions: () => Promise<void>;
  /** Fetch the unified list of ALL sessions from /api/sessions/all. */
  fetchAllSessions: () => Promise<void>;
  createSession: () => Promise<void>;
  removeSession: (id: string) => Promise<void>;
  renameSession: (id: string, title: string) => Promise<void>;
  fetchFiles: (path?: string) => Promise<void>;
}

export const useProjectStore = create<ProjectState>((set, get) => ({
  projects: [],
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
  activeMissionProject: null,
  isSkillSession: false,
  activeSkillName: null,
  agentTreesByProject: {},
  currentPath: '',
  newProjectPath: '',
  showAddProject: false,

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
  setActiveMissionProject: (project) => set({ activeMissionProject: project }),
  setIsSkillSession: (val) => set({ isSkillSession: val }),
  setActiveSkillName: (name) => set({ activeSkillName: name }),
  setNewProjectPath: (path) => set({ newProjectPath: path }),
  setShowAddProject: (show) => set({ showAddProject: show }),
  setCurrentPath: (path) => set({ currentPath: path }),

  fetchProjects: async () => {
    try {
      const resp = await dedupFetch('/api/projects');
      const data: ProjectInfo[] = await resp.json();
      set((s) => {
        const valid = new Set(data.map((p) => p.path));
        const agentTreesByProject: Record<string, Record<string, AgentTreeItem>> = {};
        for (const [path, tree] of Object.entries(s.agentTreesByProject)) {
          if (valid.has(path)) agentTreesByProject[path] = tree;
        }
        const selectedProjectRoot =
          data.length === 0
            ? ''
            : s.selectedProjectRoot && data.some((p) => p.path === s.selectedProjectRoot)
              ? s.selectedProjectRoot
              : data[0].path;
        return { projects: data, agentTreesByProject, selectedProjectRoot };
      });
    } catch (e) {
      console.error('Error fetching projects:', e);
    }
  },

  addProject: async () => {
    const { newProjectPath, fetchProjects } = get();
    if (!newProjectPath.trim()) return;
    try {
      await fetch('/api/projects', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path: newProjectPath }),
      });
      set({ newProjectPath: '', showAddProject: false });
      fetchProjects();
    } catch (e) {
      console.error('Error adding project:', e);
    }
  },

  removeProject: async (path) => {
    if (!confirm(`Are you sure you want to remove project: ${path}?`)) return;
    try {
      await fetch('/api/projects', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path }),
      });
      get().fetchProjects();
    } catch (e) {
      console.error('Error removing project:', e);
    }
  },

  pickFolder: async () => {
    try {
      const resp = await fetch('/api/utils/pick-folder');
      if (resp.ok) {
        const data = await resp.json();
        if (data.path) {
          await fetch('/api/projects', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ path: data.path }),
          });
          set({ newProjectPath: '', showAddProject: false });
          get().fetchProjects();
        }
      }
    } catch (e) {
      console.error('Error picking folder:', e);
    }
  },

  fetchSessions: async () => {
    const { selectedProjectRoot } = get();
    if (!selectedProjectRoot) return;
    try {
      const resp = await dedupFetch(`/api/sessions?project_root=${encodeURIComponent(selectedProjectRoot)}&limit=50`);
      const raw = await resp.json();
      const data: SessionInfo[] = raw.sessions ?? raw ?? [];
      set((s) => {
        // Skip update if sessions list hasn't changed (prevents re-render loops from events)
        const sessionsChanged = data.length !== s.sessions.length
          || data.some((sess, i) => sess.id !== s.sessions[i]?.id);

        // Determine active session: keep current > restore from localStorage > first session
        let next = s.activeSessionId;

        if (next && data.some((sess) => sess.id === next)) {
          // Current session still valid in this project's list — keep it
        } else if (next && (s.isMissionSession || s.isSkillSession)) {
          // Mission/skill session — keep even if not in the regular sessions list
        } else if (next && s.allSessions.some((sess) => sess.id === next)) {
          // Session exists globally (e.g. user session from a different project) — keep it
        } else {
          // Try restoring from localStorage
          const persisted = typeof window !== 'undefined'
            ? window.localStorage.getItem(ACTIVE_SESSION_STORAGE_KEY) : null;
          if (persisted && data.some((sess) => sess.id === persisted)) {
            next = persisted;
          } else if (data.length > 0) {
            // Fall back to first session
            next = data[0].id;
          } else {
            next = null;
          }
          // Sync localStorage
          if (next) window.localStorage.setItem(ACTIVE_SESSION_STORAGE_KEY, next);
          else window.localStorage.removeItem(ACTIVE_SESSION_STORAGE_KEY);
        }

        const sessionChanged = next !== s.activeSessionId;
        if (!sessionsChanged && !sessionChanged) return {};
        const patch: Partial<ProjectState> = {};
        if (sessionsChanged) patch.sessions = data;
        if (sessionChanged) {
          patch.activeSessionId = next;
          // Set selectedProjectRoot from the session so the session-change effect fires
          const sess = data.find((d) => d.id === next);
          if (sess) {
            patch.selectedProjectRoot = sess.project || sess.cwd || sess.repo_path || '~';
          }
        }
        return patch;
      });
    } catch (e) {
      console.error('Failed to fetch sessions:', e);
    }
  },

  fetchAllSessions: async () => {
    try {
      const resp = await dedupFetch('/api/sessions/all');
      const raw = await resp.json();
      const data: SessionInfo[] = raw.sessions ?? [];
      set((s) => {
        const patch: Partial<ProjectState> = { allSessions: data };
        // Auto-select first session on initial load if none is active
        if (!s.activeSessionId && data.length > 0) {
          const first = data[0];
          patch.activeSessionId = first.id;
          const isMission = first.creator === 'mission';
          const isSkill = first.creator === 'skill' || (!first.project && first.skill);
          patch.isMissionSession = isMission;
          patch.isSkillSession = !!isSkill;
          patch.activeMissionId = isMission && first.mission_id ? first.mission_id : null;
          patch.activeSkillName = isSkill && first.skill ? first.skill : null;
          patch.selectedProjectRoot = first.project || first.cwd || first.repo_path || '~';
          window.localStorage.setItem(ACTIVE_SESSION_STORAGE_KEY, first.id);
        }
        return patch;
      });
    } catch (e) {
      console.error('Failed to fetch all sessions:', e);
    }
  },

  createSession: async () => {
    const { fetchSessions, fetchAllSessions } = get();
    const now = new Date();
    const title = `Chat ${now.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}, ${now.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })}`;
    try {
      const resp = await fetch('/api/sessions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ title }),
      });
      const data = await resp.json();
      set({ activeSessionId: data.id });
      fetchSessions();
      fetchAllSessions();
    } catch (e) {
      console.error('Error creating session:', e);
    }
  },

  removeSession: async (id) => {
    const { activeSessionId, allSessions, fetchSessions, fetchAllSessions } = get();
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
      if (activeSessionId === id) {
        window.localStorage.removeItem(ACTIVE_SESSION_STORAGE_KEY);
        set({ activeSessionId: null });
      }
      fetchSessions();
      fetchAllSessions();
    } catch (e) {
      console.error('Error removing session:', e);
    }
  },

  renameSession: async (id, title) => {
    const { selectedProjectRoot, fetchSessions } = get();
    if (!selectedProjectRoot) return;
    try {
      await fetch('/api/sessions', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot, session_id: id, title }),
      });
      fetchSessions();
    } catch (e) {
      console.error('Error renaming session:', e);
    }
  },

  fetchFiles: async (path = '') => {
    const { selectedProjectRoot } = get();
    if (!selectedProjectRoot) return;
    try {
      await dedupFetch(`/api/files?project_root=${encodeURIComponent(selectedProjectRoot)}&path=${encodeURIComponent(path)}`);
      set({ currentPath: path });
    } catch (e) {
      console.error('Error fetching files:', e);
    }
  },
}));
