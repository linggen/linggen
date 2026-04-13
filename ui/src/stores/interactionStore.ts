/**
 * Pending interactions between agent and user: plans, askUser, queued messages.
 */
import { create } from 'zustand';
import type { Plan, PendingAskUser, QueuedChatItem } from '../types';

interface InteractionState {
  queuedMessages: QueuedChatItem[];
  pendingPlan: Plan | null;
  pendingPlanAgentId: string | null;
  pendingAskUser: PendingAskUser | null;
  activePlan: Plan | null;

  setQueuedMessages: (updater: QueuedChatItem[] | ((prev: QueuedChatItem[]) => QueuedChatItem[])) => void;
  setPendingPlan: (plan: Plan | null | ((prev: Plan | null) => Plan | null)) => void;
  setPendingPlanAgentId: (id: string | null) => void;
  setPendingAskUser: (ask: PendingAskUser | null | ((prev: PendingAskUser | null) => PendingAskUser | null)) => void;
  setActivePlan: (plan: Plan | null | ((prev: Plan | null) => Plan | null)) => void;
}

export const useInteractionStore = create<InteractionState>((set) => ({
  queuedMessages: [],
  pendingPlan: null,
  pendingPlanAgentId: null,
  pendingAskUser: null,
  activePlan: null,

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
  setActivePlan: (updater) => set((s) => ({
    activePlan: typeof updater === 'function' ? updater(s.activePlan) : updater,
  })),
}));
