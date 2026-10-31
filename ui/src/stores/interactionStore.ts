/**
 * Pending interactions between agent and user: plans, askUser, queued messages.
 */
import { create } from 'zustand';
import type { BackgroundTask, Plan, PendingAskUser, QueuedChatItem } from '../types';
import { enqueueAsk, removeAsk, replaceAsks, type AskQueues } from '../lib/askQueue.mts';

interface InteractionState {
  queuedMessages: QueuedChatItem[];
  /** The active session's background tasks (doc/background-tasks-spec.md). */
  backgroundTasks: BackgroundTask[];
  pendingPlan: Plan | null;
  pendingPlanAgentId: string | null;
  askQueues: AskQueues;
  askRevision: number;
  enqueueAsk: (sessionId: string, ask: PendingAskUser) => void;
  removeAsk: (sessionId: string, questionId: string) => void;
  replaceAsks: (sessionId: string, asks: PendingAskUser[]) => void;
  activePlan: Plan | null;

  setQueuedMessages: (updater: QueuedChatItem[] | ((prev: QueuedChatItem[]) => QueuedChatItem[])) => void;
  setBackgroundTasks: (tasks: BackgroundTask[]) => void;
  setPendingPlan: (plan: Plan | null | ((prev: Plan | null) => Plan | null)) => void;
  setPendingPlanAgentId: (id: string | null) => void;
  setActivePlan: (plan: Plan | null | ((prev: Plan | null) => Plan | null)) => void;
}

export const useInteractionStore = create<InteractionState>((set) => ({
  queuedMessages: [],
  backgroundTasks: [],
  pendingPlan: null,
  pendingPlanAgentId: null,
  askQueues: {},
  askRevision: 0,
  enqueueAsk: (sessionId, ask) => set((s) => ({ askQueues: enqueueAsk(s.askQueues, sessionId, ask), askRevision: s.askRevision + 1 })),
  removeAsk: (sessionId, questionId) => set((s) => ({ askQueues: removeAsk(s.askQueues, sessionId, questionId), askRevision: s.askRevision + 1 })),
  replaceAsks: (sessionId, asks) => set((s) => ({ askQueues: replaceAsks(s.askQueues, sessionId, asks), askRevision: s.askRevision + 1 })),
  activePlan: null,

  setQueuedMessages: (updater) => set((s) => ({
    queuedMessages: typeof updater === 'function' ? updater(s.queuedMessages) : updater,
  })),
  setBackgroundTasks: (tasks) => set({ backgroundTasks: tasks }),
  setPendingPlan: (updater) => set((s) => ({
    pendingPlan: typeof updater === 'function' ? updater(s.pendingPlan) : updater,
  })),
  setPendingPlanAgentId: (id) => set({ pendingPlanAgentId: id }),
  setActivePlan: (updater) => set((s) => ({
    activePlan: typeof updater === 'function' ? updater(s.activePlan) : updater,
  })),
}));
