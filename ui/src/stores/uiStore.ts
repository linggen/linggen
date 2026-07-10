/**
 * UI navigation, overlays, and transient chrome state.
 */
import { create } from 'zustand';
import type { CronMission } from '../types';

/** Top-level view discriminator. Drives MainApp vs ConsumerApp selection
 *  in the entry router; page navigation within the main view is now handled
 *  by react-router-dom routes. */
export type Page = 'main' | 'consumer';
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

/** Yinyue's current spoken line — shown as a transient in-character bubble,
 *  driven by the `yinyue_speak` event (paired with her voice). */
export interface YinyueSpeech {
  id: string;
  text: string;
  emotion: string;
}

/** A pet expression directive (emotion and/or one-shot gesture), driven by
 *  the `pet_express` event. Generic across pets/mascots. */
export interface PetExpress {
  id: string;
  emotion?: string;
  action?: string;
}

interface UiState {
  // Page navigation
  currentPage: Page;
  sidebarTab: SidebarTab;

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

  // Session permission mode (chat/read/edit/admin — pushed by server page_state)
  sessionMode: string | null;

  // Chat UI
  verboseMode: boolean;
  copyChatStatus: 'idle' | 'copied' | 'error';

  // Toasts
  toasts: Toast[];
  addToast: (toast: Omit<Toast, 'id'>) => void;
  removeToast: (id: string) => void;

  // Yinyue speech bubble (driven by the `yinyue_speak` event)
  yinyueSpeech: YinyueSpeech | null;
  showYinyueSpeech: (text: string, emotion: string) => void;
  clearYinyueSpeech: () => void;

  // Pet expression (driven by the `pet_express` event)
  petExpress: PetExpress | null;
  pushPetExpress: (emotion?: string, action?: string) => void;

  // Pet thinking — true while a reply is in flight; drives the pondering pose.
  petThinking: boolean;
  setPetThinking: (v: boolean) => void;

  // Pet speaking — true while her voice is playing; drives the talking body loop.
  petSpeaking: boolean;
  setPetSpeaking: (v: boolean) => void;

  // Yinyue presenter — true when THIS surface holds the server's FCFS singleton
  // lock and should render her avatar + bubble + play her voice. Set by the
  // `yinyue_present` control push. Others stay false (blank + silent).
  yinyuePresenter: boolean;
  setYinyuePresenter: (v: boolean) => void;

  // Actions
  setCurrentPage: (page: Page) => void;
  setSidebarTab: (tab: SidebarTab) => void;
  openMissionEditor: (mission: CronMission | null) => void;
  closeMissionEditor: () => void;
  bumpMissionRefreshKey: () => void;

  setOverlay: (overlay: string | null) => void;
  setModelPickerOpen: (open: boolean) => void;
  setSessionModel: (model: string | null) => void;
  setSessionMode: (mode: string | null) => void;
  setShowAgentSpecEditor: (show: boolean) => void;
  setOpenApp: (app: AppPanelState | null) => void;

  setSelectedFileContent: (content: string | null) => void;
  setSelectedFilePath: (path: string | null) => void;
  closeFilePreview: () => void;

  setVerboseMode: (mode: boolean) => void;
  setCopyChatStatus: (status: 'idle' | 'copied' | 'error') => void;
}

const VERBOSE_MODE_STORAGE_KEY = 'linggen:verbose-mode';

export const useUiStore = create<UiState>((set) => ({
  currentPage: 'main',
  sidebarTab: 'projects',
  editingMission: null,
  missionRefreshKey: 0,
  overlay: null,
  modelPickerOpen: false,
  sessionModel: null,
  sessionMode: null,
  showAgentSpecEditor: false,
  openApp: null,
  selectedFileContent: null,
  selectedFilePath: null,
  verboseMode: typeof window !== 'undefined' ? window.localStorage.getItem(VERBOSE_MODE_STORAGE_KEY) === 'true' : false,
  copyChatStatus: 'idle',
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

  yinyueSpeech: null,
  showYinyueSpeech: (text, emotion) => {
    const id = `ys-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    set({ yinyueSpeech: { id, text, emotion } });
    // Linger well past the spoken line — her heralds often fire while the
    // user is on another tab, and the bubble must still be there after
    // they switch over. Click dismisses anytime; superseded speech wins.
    const ms = Math.min(45000, Math.max(15000, 2500 + text.length * 55));
    setTimeout(() => {
      set((s) => (s.yinyueSpeech?.id === id ? { yinyueSpeech: null } : {}));
    }, ms);
  },
  clearYinyueSpeech: () => set({ yinyueSpeech: null }),

  yinyuePresenter: false,
  setYinyuePresenter: (v) => set({ yinyuePresenter: v }),

  petExpress: null,
  pushPetExpress: (emotion, action) => {
    const id = `px-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    set({ petExpress: { id, emotion, action } });
  },

  petThinking: false,
  setPetThinking: (v) => set({ petThinking: v }),

  petSpeaking: false,
  setPetSpeaking: (v) => set({ petSpeaking: v }),

  setCurrentPage: (page) => set({ currentPage: page }),
  setSidebarTab: (tab) => set({ sidebarTab: tab }),
  openMissionEditor: (mission) => set({ editingMission: mission }),
  closeMissionEditor: () => set({ editingMission: null }),
  bumpMissionRefreshKey: () => set((s) => ({ missionRefreshKey: s.missionRefreshKey + 1 })),

  setOverlay: (overlay) => set({ overlay }),
  setModelPickerOpen: (open) => set({ modelPickerOpen: open }),
  setSessionModel: (model) => set({ sessionModel: model }),
  setSessionMode: (mode: string | null) => set({ sessionMode: mode }),
  setShowAgentSpecEditor: (show) => set({ showAgentSpecEditor: show }),
  setOpenApp: (app) => set({ openApp: app }),

  setSelectedFileContent: (content) => set({ selectedFileContent: content }),
  setSelectedFilePath: (path) => set({ selectedFilePath: path }),
  closeFilePreview: () => set({ selectedFileContent: null, selectedFilePath: null }),

  setVerboseMode: (mode) => {
    window.localStorage.setItem(VERBOSE_MODE_STORAGE_KEY, String(mode));
    set({ verboseMode: mode });
  },
  setCopyChatStatus: (status) => set({ copyChatStatus: status }),
}));
