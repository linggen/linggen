/**
 * UI navigation & transient state.
 */
import { create } from 'zustand';
import type { CronMission, ManagementTab, Plan, PendingAskUser, QueuedChatItem } from '../types';

export type Page = 'main' | 'settings' | 'mission-editor';
export type SidebarTab = 'projects' | 'missions';

export interface Toast {
  id: string;
  message: string;
  variant: 'success' | 'error' | 'info';
  /** Auto-dismiss after this many ms (default 5000). 0 = no auto-dismiss. */
  duration?: number;
  /** Optional callback when the toast is clicked. */
  onClick?: () => void;
}

export interface AppPanelState {
  skill: string;
  launcher: string;
  url: string;
  title: string;
  width?: number;
  height?: number;
}

interface UiState {
  // Page navigation
  currentPage: Page;
  sidebarTab: SidebarTab;
  initialSettingsTab: ManagementTab | undefined;

  // Mission editor
  editingMission: CronMission | null;
  missionRefreshKey: number;

  // Overlays & modals
  overlay: string | null;
  modelPickerOpen: boolean;
  showAgentSpecEditor: boolean;
  openApp: AppPanelState | null;

  // File preview
  selectedFileContent: string | null;
  selectedFilePath: string | null;

  // Session-level model override (not persisted — resets on session change)
  sessionModel: string | null;

  // Chat UI
  queuedMessages: QueuedChatItem[];
  pendingPlan: Plan | null;
  pendingPlanAgentId: string | null;
  pendingAskUser: PendingAskUser | null;
  activePlan: Plan | null;
  verboseMode: boolean;
  copyChatStatus: 'idle' | 'copied' | 'error';

  // SSE connection status
  sseStatus: 'connected' | 'reconnecting';
  setSseStatus: (status: 'connected' | 'reconnecting') => void;

  // Toasts
  toasts: Toast[];
  addToast: (toast: Omit<Toast, 'id'>) => void;
  removeToast: (id: string) => void;

  // Actions
  setCurrentPage: (page: Page) => void;
  setSidebarTab: (tab: SidebarTab) => void;
  setInitialSettingsTab: (tab: ManagementTab | undefined) => void;
  openSettings: (tab?: ManagementTab) => void;
  openMissionEditor: (mission: CronMission | null) => void;
  closeMissionEditor: () => void;
  bumpMissionRefreshKey: () => void;

  setOverlay: (overlay: string | null) => void;
  setModelPickerOpen: (open: boolean) => void;
  setSessionModel: (model: string | null) => void;
  setShowAgentSpecEditor: (show: boolean) => void;
  setOpenApp: (app: AppPanelState | null) => void;

  setSelectedFileContent: (content: string | null) => void;
  setSelectedFilePath: (path: string | null) => void;
  closeFilePreview: () => void;

  setQueuedMessages: (updater: QueuedChatItem[] | ((prev: QueuedChatItem[]) => QueuedChatItem[])) => void;
  setPendingPlan: (plan: Plan | null | ((prev: Plan | null) => Plan | null)) => void;
  setPendingPlanAgentId: (id: string | null) => void;
  setPendingAskUser: (ask: PendingAskUser | null | ((prev: PendingAskUser | null) => PendingAskUser | null)) => void;
  fetchPendingAskUser: () => Promise<void>;
  setActivePlan: (plan: Plan | null | ((prev: Plan | null) => Plan | null)) => void;
  setVerboseMode: (mode: boolean) => void;
  setCopyChatStatus: (status: 'idle' | 'copied' | 'error') => void;
}

const VERBOSE_MODE_STORAGE_KEY = 'linggen:verbose-mode';

export const useUiStore = create<UiState>((set) => ({
  currentPage: 'main',
  sidebarTab: 'projects',
  initialSettingsTab: undefined,
  editingMission: null,
  missionRefreshKey: 0,
  overlay: null,
  modelPickerOpen: false,
  sessionModel: null,
  showAgentSpecEditor: false,
  openApp: null,
  selectedFileContent: null,
  selectedFilePath: null,
  queuedMessages: [],
  pendingPlan: null,
  pendingPlanAgentId: null,
  pendingAskUser: null,
  activePlan: null,
  verboseMode: typeof window !== 'undefined' ? window.localStorage.getItem(VERBOSE_MODE_STORAGE_KEY) === 'true' : false,
  copyChatStatus: 'idle',
  sseStatus: 'connected',
  setSseStatus: (status) => set({ sseStatus: status }),
  toasts: [],
  addToast: (toast) => {
    const id = `toast-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    const duration = toast.duration ?? 5000;
    set((s) => ({ toasts: [...s.toasts, { ...toast, id }] }));
    if (duration > 0) {
      setTimeout(() => {
        set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
      }, duration);
    }
  },
  removeToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

  setCurrentPage: (page) => set({ currentPage: page }),
  setSidebarTab: (tab) => set({ sidebarTab: tab }),
  setInitialSettingsTab: (tab) => set({ initialSettingsTab: tab }),
  openSettings: (tab) => set({ currentPage: 'settings', initialSettingsTab: tab }),
  openMissionEditor: (mission) => set({ editingMission: mission, currentPage: 'mission-editor' }),
  closeMissionEditor: () => set({ editingMission: null, currentPage: 'main' }),
  bumpMissionRefreshKey: () => set((s) => ({ missionRefreshKey: s.missionRefreshKey + 1 })),

  setOverlay: (overlay) => set({ overlay }),
  setModelPickerOpen: (open) => set({ modelPickerOpen: open }),
  setSessionModel: (model) => set({ sessionModel: model }),
  setShowAgentSpecEditor: (show) => set({ showAgentSpecEditor: show }),
  setOpenApp: (app) => set({ openApp: app }),

  setSelectedFileContent: (content) => set({ selectedFileContent: content }),
  setSelectedFilePath: (path) => set({ selectedFilePath: path }),
  closeFilePreview: () => set({ selectedFileContent: null, selectedFilePath: null }),

  setQueuedMessages: (updater) => set((s) => ({
    queuedMessages: typeof updater === 'function' ? updater(s.queuedMessages) : updater,
  })),
  setPendingPlan: (updater) => set((s) => ({
    pendingPlan: typeof updater === 'function' ? updater(s.pendingPlan) : updater,
  })),
  setPendingPlanAgentId: (id) => set({ pendingPlanAgentId: id }),
  setPendingAskUser: (updater) => set((s) => ({
    pendingAskUser: typeof updater === 'function' ? updater(s.pendingAskUser) : updater,
  })),
  fetchPendingAskUser: async () => {
    try {
      const resp = await fetch('/api/pending-ask-user');
      if (!resp.ok) return;
      const items = await resp.json();
      if (Array.isArray(items) && items.length > 0) {
        // Get current active session from project store
        const { useProjectStore } = await import('./projectStore');
        const activeSessionId = useProjectStore.getState().activeSessionId;
        // Only show ask_user for the current session, or if no session is active
        // show ask_user items that have no session_id (legacy/global)
        const filtered = items.filter((item: { session_id?: string | null }) => {
          if (!activeSessionId) return !item.session_id; // main page: only show global
          return item.session_id === activeSessionId || !item.session_id;
        });
        if (filtered.length > 0) {
          const item = filtered[0];
          set({
            pendingAskUser: {
              questionId: item.question_id,
              agentId: item.agent_id,
              questions: item.questions,
            },
          });
        } else {
          set({ pendingAskUser: null });
        }
      } else {
        set({ pendingAskUser: null });
      }
    } catch { /* ignore */ }
  },
  setActivePlan: (updater) => set((s) => ({
    activePlan: typeof updater === 'function' ? updater(s.activePlan) : updater,
  })),
  setVerboseMode: (mode) => {
    window.localStorage.setItem(VERBOSE_MODE_STORAGE_KEY, String(mode));
    set({ verboseMode: mode });
  },
  setCopyChatStatus: (status) => set({ copyChatStatus: status }),
}));
