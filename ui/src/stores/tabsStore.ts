/**
 * App tabs for the unified Linggen launcher. The shell opens MainApp as the
 * "Linggen home"; the tab bar turns it into a multi-app workspace — the Ling
 * chat is the permanent first tab, and each opened skill app becomes a tab.
 * App tab iframes stay mounted (hidden when inactive) so their state survives
 * switching. Tabs are in-memory only (not persisted across restart) for v1.
 */
import { create } from 'zustand';

export interface AppTab {
  id: string;
  kind: 'chat' | 'app';
  title: string;
  skill?: string; // kind:'app' only
  url?: string; // iframe src, kind:'app' only
}

const CHAT_TAB: AppTab = { id: 'chat', kind: 'chat', title: 'Ling' };

interface TabsState {
  tabs: AppTab[];
  activeTabId: string;
  /** Open a skill app as a tab (focus it if already open). */
  openAppTab: (skill: string, title: string, url: string) => void;
  closeTab: (id: string) => void;
  setActiveTab: (id: string) => void;
}

export const useTabsStore = create<TabsState>((set, get) => ({
  tabs: [CHAT_TAB],
  activeTabId: 'chat',

  openAppTab: (skill, title, url) => {
    const existing = get().tabs.find((t) => t.kind === 'app' && t.skill === skill);
    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }
    const id = `app-${skill}`;
    set((s) => ({ tabs: [...s.tabs, { id, kind: 'app', skill, title, url }], activeTabId: id }));
  },

  closeTab: (id) =>
    set((s) => {
      if (id === 'chat') return s; // the chat tab is permanent
      const tabs = s.tabs.filter((t) => t.id !== id);
      const activeTabId = s.activeTabId === id ? 'chat' : s.activeTabId;
      return { tabs, activeTabId };
    }),

  setActiveTab: (id) => set({ activeTabId: id }),
}));
