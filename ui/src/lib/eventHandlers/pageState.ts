/**
 * page_state — aggregated state push from the server over the control channel.
 * Replaces individual HTTP polling for session list, models, skills, etc.
 */
import type { UiEvent } from '../../types';
import { useSessionStore } from '../../stores/sessionStore';
import { useServerStore } from '../../stores/serverStore';
import { useUserStore } from '../../stores/userStore';
import { useUiStore } from '../../stores/uiStore';
import { useInteractionStore } from '../../stores/interactionStore';
import { useChatStore } from '../../stores/chatStore';

export function handlePageState(item: UiEvent): void {
  const ps = item.data;
  if (!ps) return;

  applyPermission(ps);
  applyGlobalLists(ps);
  applyPendingAskUser(ps);
  applyBusySessions(ps);
  applyScopedState(ps);
}

function applyPermission(ps: any): void {
  // userType is set once by user_info at connection time, not by page_state.
  if (!ps.permission) return;
  const userStore = useUserStore.getState();
  // For owners, don't let page_state overwrite room_name — it's fetched via HTTP
  // from linggen.dev and page_state doesn't have it (owner's UserContext.room_name is null).
  const roomName = userStore.userType === 'owner' ? userStore.userRoomName : (ps.room_name ?? null);
  if (userStore.userPermission !== ps.permission || userStore.userRoomName !== roomName) {
    userStore.setUserInfo(ps.permission, roomName, userStore.userTokenBudget);
    useUiStore.getState().setCurrentPage(userStore.userType === 'consumer' ? 'consumer' : 'main');
  }
  // Room enabled status (owner only — pushed from room_config.toml)
  if (ps.room_enabled !== undefined && ps.room_enabled !== null) {
    useUserStore.getState().setRoomEnabled(ps.room_enabled);
  }
}

function applyGlobalLists(ps: any): void {
  if (ps.all_sessions) {
    useSessionStore.setState({ allSessions: ps.all_sessions });
    // Auto-select session if none is active (e.g. on init/restart):
    // 1. Try to restore from localStorage (last used session)
    // 2. Fall back to first session in the list
    const store = useSessionStore.getState();
    if (!store.activeSessionId && ps.all_sessions.length > 0) {
      const saved = window.localStorage.getItem('linggen:active-session');
      const match = saved && ps.all_sessions.find((s: any) => s.id === saved);
      store.setActiveSessionId(match ? saved! : ps.all_sessions[0].id);
    }
  }
  if (ps.models) useServerStore.setState({ models: ps.models });
  if (ps.default_models) useServerStore.setState({ defaultModels: ps.default_models });
  if (ps.skills) useServerStore.setState({ skills: ps.skills });
  if (ps.missions) useUiStore.getState().bumpMissionRefreshKey();
}

function applyPendingAskUser(ps: any): void {
  if (ps.pending_ask_user === undefined) return;
  // Restore pending ask-user from server state — only for the active session.
  // Without session filtering, prompts from other sessions leak into skill iframes.
  const activeSessionId = useSessionStore.getState().activeSessionId;
  const items = (Array.isArray(ps.pending_ask_user) ? ps.pending_ask_user : [])
    .filter((it: any) => !it.session_id || it.session_id === activeSessionId);
  const interaction = useInteractionStore.getState();
  if (items.length > 0 && !interaction.pendingAskUser) {
    const first = items[0];
    interaction.setPendingAskUser({
      questionId: first.question_id,
      agentId: first.agent_id || '',
      questions: first.questions || [],
    });
  }
}

function applyBusySessions(ps: any): void {
  if (!ps.busy_sessions) return;
  // Merge busy_sessions into agentStatus so session list shows spinners.
  // Only set status for sessions not already tracked (real-time activity events
  // are authoritative for the active session).
  useServerStore.getState().setAgentStatus((prev) => {
    const next = { ...prev };
    // Clear sessions that are no longer busy
    for (const sid of Object.keys(next)) {
      if (!(sid in ps.busy_sessions) && next[sid] !== 'idle') {
        next[sid] = 'idle';
      }
    }
    // Add/update busy sessions
    for (const [sid, status] of Object.entries(ps.busy_sessions)) {
      if (!next[sid] || next[sid] === 'idle') {
        next[sid] = status as any;
      }
    }
    return next;
  });
}

function applyScopedState(ps: any): void {
  if (ps.agents) {
    useServerStore.setState({ agents: ps.agents });
    // Auto-select the first agent on a fresh client where no agent has
    // been picked yet (empty localStorage). Without this, chat send bails
    // silently because useChatActions.sendChatMessage early-returns when
    // selectedAgent is "" — and the user-message bubble never even appears.
    const store = useServerStore.getState();
    if (!store.selectedAgent && Array.isArray(ps.agents) && ps.agents.length > 0) {
      store.setSelectedAgent(ps.agents[0].name);
    }
  }
  if (ps.agent_runs) {
    // Skip update if runs haven't changed (prevents re-render loops)
    const prev = useServerStore.getState().agentRuns;
    const data = Array.isArray(ps.agent_runs) ? ps.agent_runs : [];
    if (
      data.length !== prev.length ||
      !data.every((r: any, i: number) => r.run_id === prev[i]?.run_id && r.status === prev[i]?.status)
    ) {
      useServerStore.setState({ agentRuns: data });
    }
    reconcileStreamingState(data);
  }
  if (ps.sessions) useSessionStore.setState({ sessions: ps.sessions });

  if (!ps.session_permission) return;
  const perm = ps.session_permission;
  if (perm.effective_mode) {
    useUiStore.getState().setSessionMode(perm.effective_mode);
  }
}

/** Grace before server-authoritative "idle" may retire optimistic client
 *  state — long enough to cover send→run/begin→page_state propagation. */
const STREAM_STATE_STALE_MS = 15_000;

/**
 * Terminal turn events (message / turn_complete / run outcome) are only
 * delivered while the session's data channel is open, and the server buffers
 * them for at most 60s after it closes. A mid-turn session switch that lasts
 * longer loses them permanently — leaving a pendingSends flag that spins the
 * "Thinking…" bar forever and a frozen isGenerating bubble that later
 * double-renders next to its persisted row. page_state's agent_runs is
 * server-authoritative, so use each push to retire streaming state that the
 * runs say cannot still be live.
 */
function reconcileStreamingState(runs: any[]): void {
  const running = new Set(
    runs
      .filter((r: any) => !r.parent_run_id && r.status === 'running')
      .map((r: any) => r.session_id),
  );
  const serverStore = useServerStore.getState();
  const now = Date.now();
  for (const [sid, sentAt] of Object.entries(serverStore.pendingSends)) {
    if (!running.has(sid) && now - sentAt > STREAM_STATE_STALE_MS) {
      serverStore.setPendingSend(sid, false);
    }
  }

  // Frozen generating bubbles in the visible session: finalize them so the
  // next syncPersisted merge collapses each into its persisted row instead
  // of rendering both. Age-gated — a bubble still receiving tokens has a
  // fresh timestampMs and is left alone.
  const chatStore = useChatStore.getState();
  const activeSessionId = useSessionStore.getState().activeSessionId;
  if (!activeSessionId || running.has(activeSessionId)) return;
  const pendingTs = serverStore.pendingSends[activeSessionId];
  if (pendingTs && now - pendingTs <= STREAM_STATE_STALE_MS) return;
  chatStore.finalizeAllGenerating(STREAM_STATE_STALE_MS);
}
