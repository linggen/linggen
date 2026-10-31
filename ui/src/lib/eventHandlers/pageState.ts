/**
 * page_state — aggregated state push from the server over the control channel.
 * Replaces individual HTTP polling for session list, models, skills, etc.
 */
import type { AgentRunInfo } from '../../types';
import type { UiEventOf, PageState } from '../../types';
import { useSessionStore } from '../../stores/sessionStore';
import { useServerStore, isSessionBusy } from '../../stores/serverStore';
import { useUserStore } from '../../stores/userStore';
import { useUiStore } from '../../stores/uiStore';
import { useInteractionStore } from '../../stores/interactionStore';
import { useChatStore } from '../../stores/chatStore';
import { UNSPOKEN_SENDERS } from '../messageUtils';
import { interruptedResend, INTERRUPTED_TEXT } from '../interruptedTurn.mts';
import { stillOn } from '../sessionLoad.mts';
import { sessionRootOf } from '../sessionRoot.mts';
import { isPermissionSuppressed } from './_shared';
import { noteUiBuild, type PageFacts } from '../uiBuild.mts';


export function handlePageState(item: UiEventOf<'page_state'>): void {
  const ps = item.data;
  if (!ps) return;

  applyPermission(ps);
  applyGlobalLists(ps);
  applyPendingAskUser(ps);
  applyBusySessions(ps);
  applyQueued(ps);
  applyTasks(ps);
  applyScopedState(ps);
  noteUiBuild(ps.ui_build, pageFacts);
}

/** What would be cut short or lost if the page reloaded now. */
function pageFacts(): PageFacts {
  const sid = useSessionStore.getState().activeSessionId;
  const { pendingPlan, askQueues } = useInteractionStore.getState();
  const ui = useUiStore.getState();
  return {
    replying: isSessionBusy(useServerStore.getState(), sid),
    awaitingAnswer: !!pendingPlan || (!!sid && (askQueues[sid]?.length ?? 0) > 0),
    dialogOpen: ui.modelPickerOpen || ui.showAgentSpecEditor || !!ui.openApp || !!ui.selectedFilePath,
  };
}

function applyPermission(ps: PageState): void {
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

function applyGlobalLists(ps: PageState): void {
  if (ps.all_sessions) {
    useSessionStore.setState({ allSessions: ps.all_sessions, allSessionsLoaded: true });
    // Auto-select session if none is active (e.g. on init/restart):
    // 1. Try to restore from localStorage (last used session)
    // 2. Fall back to first session in the list
    const store = useSessionStore.getState();
    if (!store.activeSessionId && ps.all_sessions.length > 0) {
      const saved = window.localStorage.getItem('linggen:active-session');
      const pick = ps.all_sessions.find((s) => s.id === saved) || ps.all_sessions[0];
      store.setActiveSessionId(pick.id);
      // The restored session's own folder — never one kept from another.
      store.setSelectedProjectRoot(sessionRootOf(pick));
    }
  }
  if (ps.models) useServerStore.setState({ models: ps.models, modelsLoaded: true });
  if (ps.default_models) useServerStore.setState({ defaultModels: ps.default_models });
  if (ps.claude_default_model !== undefined) useServerStore.setState({ claudeDefaultModel: ps.claude_default_model });
  if (ps.skills) useServerStore.setState({ skills: ps.skills, skillsLoaded: true });
  if (ps.missions) useUiStore.getState().bumpMissionRefreshKey();
}

function applyPendingAskUser(ps: PageState): void {
  if (ps.pending_ask_user === undefined) return;
  const items = Array.isArray(ps.pending_ask_user) ? ps.pending_ask_user : [];
  const interaction = useInteractionStore.getState();
  const sessions = new Set([...Object.keys(interaction.askQueues), ...items.flatMap((it) => it.session_id ? [it.session_id] : [])]);
  for (const sid of sessions) {
    const incoming = items.filter((it) => it.session_id === sid).map((it) => ({
      questionId: it.question_id, agentId: it.agent_id || '', questions: it.questions || [],
    }));
    // Server iteration order is arbitrary; retain the order already displayed.
    const old = interaction.askQueues[sid] ?? [];
    const ordered = [...old.flatMap((ask) => incoming.filter((it) => it.questionId === ask.questionId)),
      ...incoming.filter((it) => !old.some((ask) => ask.questionId === it.questionId))];
    interaction.replaceAsks(sid, ordered);
  }
}

function applyBusySessions(ps: PageState): void {
  if (!ps.busy_sessions) return;
  useServerStore.getState().setBusySessions(ps.busy_sessions);
}

/// The server's authoritative queue for the scoped session, as
/// `[{agent_id, items}]` — already scoped to this peer's view context, and
/// flattened here because the store holds one list whose items carry their own
/// `agent_id` (ChatPanel filters by the selected agent).
///
/// Deliberately applies an EMPTY array too: an absent agent means nothing is
/// queued, and that is the case that used to strand the composer. A `queue`
/// event only reaches a client that was listening, and there is no endpoint to
/// re-read the queue — so a client that missed the drain (channel closed,
/// reconnected as a fresh peer) kept showing a queue the server had already
/// thrown away, and refused every new message as "agent is busy".
function applyQueued(ps: PageState): void {
  if (ps.queued === undefined) return;
  const items = (Array.isArray(ps.queued) ? ps.queued : []).flatMap((entry) =>
    Array.isArray(entry?.items) ? entry.items : []
  );
  useInteractionStore.getState().setQueuedMessages(items);
}

/** The session's background tasks; an absent list on a scoped push is none. */
function applyTasks(ps: PageState): void {
  if (ps.queued === undefined) return;
  useInteractionStore.getState().setBackgroundTasks(Array.isArray(ps.tasks) ? ps.tasks : []);
}

function applyScopedState(ps: PageState): void {
  if (ps.agents || ps.agents_met) {
    useServerStore.getState().applyAgentPresence({ agents: ps.agents, agentsMet: ps.agents_met });
  }
  if (ps.agent_runs) {
    // Skip update if runs haven't changed (prevents re-render loops)
    const prev = useServerStore.getState().agentRuns;
    const data = Array.isArray(ps.agent_runs) ? ps.agent_runs : [];
    if (
      data.length !== prev.length ||
      !data.every((r, i) => r.run_id === prev[i]?.run_id && r.status === prev[i]?.status)
    ) {
      useServerStore.setState({ agentRuns: data });
    }
    reconcileStreamingState(data, ps.session_id);
    if (reconcilesSubagents(ps)) useChatStore.getState().reconcileSubagents(data.filter((run) => run.session_id === ps.session_id));
  }
  if (ps.sessions) useSessionStore.setState({ sessions: ps.sessions });

  if (!ps.session_permission) return;
  const perm = ps.session_permission;
  const all = perm.path_modes?.find((grant) => grant.path === '*');
  useUiStore.getState().setSessionAllMode(all?.mode ?? null);
  // A mode the user just picked wins until its PATCH has propagated.
  if (perm.effective_mode && !isPermissionSuppressed()) {
    useUiStore.getState().setSessionMode(perm.effective_mode);
  }
}

/** The runs settle the visible session's subagent rows only when the push
 *  is for that session, and the session is known to run on the engine — a
 *  Claude session's children are the SDK's, not engine runs. */
function reconcilesSubagents(ps: PageState): boolean {
  return isForActiveSession(ps) && ps.session_runtime === 'linggen';
}

/** The push's scoped part describes the session on screen (not the one
 *  being left during a switch). */
function isForActiveSession(ps: PageState): boolean {
  const sid = useSessionStore.getState().activeSessionId;
  return !!sid && ps.session_id === sid;
}

/** Grace before server-authoritative "idle" may retire optimistic client
 *  state — long enough to cover send→run/begin→page_state propagation. */
const STREAM_STATE_STALE_MS = 15_000;

/** The visible session, when it was busy as the link dropped. Reconnect
 *  wipes the optimistic pendingSends before any page_state can judge them,
 *  so without this a turn that died with the engine was never called
 *  interrupted — the chat just showed "Musing for 58s" as if it finished. */
let busyBeforeReconnect: string | null = null;

/** Called on reconnect, BEFORE the client's optimistic flags are cleared. */
export function armInterruptCheck(): void {
  const sid = useSessionStore.getState().activeSessionId;
  const { pendingSends, agentRuns } = useServerStore.getState();
  const busy = !!sid && (
    !!pendingSends[sid] ||
    agentRuns.some((r) => r.session_id === sid && !r.parent_run_id && r.status === 'running')
  );
  busyBeforeReconnect = busy ? sid : null;
}

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
function reconcileStreamingState(runs: AgentRunInfo[], runsFor: string | undefined): void {
  const running = new Set(
    runs
      .filter((r) => !r.parent_run_id && r.status === 'running')
      .map((r) => r.session_id),
  );
  // The first runs list after a reconnect settles the turn that was busy when
  // the link dropped: still running means it survived a blip; absent means it
  // died, and the persisted thread decides whether anything answered.
  // Runs listed for another session say nothing about this one.
  if (busyBeforeReconnect && busyBeforeReconnect === runsFor) {
    const sid = busyBeforeReconnect;
    busyBeforeReconnect = null;
    if (!running.has(sid)) void markInterruptedIfUnanswered(sid);
  }

  const serverStore = useServerStore.getState();
  const now = Date.now();
  for (const [sid, sentAt] of Object.entries(serverStore.pendingSends)) {
    if (!running.has(sid) && now - sentAt > STREAM_STATE_STALE_MS) {
      serverStore.setPendingSend(sid, false);
      void markInterruptedIfUnanswered(sid);
    }
  }

  // Frozen generating bubbles in the visible session: finalize them so the
  // next syncPersisted merge collapses each into its persisted row instead
  // of rendering both. Age-gated — a bubble still receiving tokens has a
  // fresh timestampMs and is left alone.
  const chatStore = useChatStore.getState();
  const activeSessionId = useSessionStore.getState().activeSessionId;
  if (!activeSessionId || activeSessionId !== runsFor || running.has(activeSessionId)) return;
  const pendingTs = serverStore.pendingSends[activeSessionId];
  if (pendingTs && now - pendingTs <= STREAM_STATE_STALE_MS) return;
  chatStore.finalizeAllGenerating(STREAM_STATE_STALE_MS);
}

/**
 * A retired pendingSend means the server no longer runs this session's turn,
 * yet no TurnComplete ever arrived. Two different truths look identical at
 * that moment: the turn finished while our channel was closed (the reply is
 * persisted — re-sync renders it), or the daemon was killed/restarted
 * mid-run and the reply never existed. Re-read the persisted session and
 * only when the last message is still the user's, say so — otherwise the
 * chat just ends on the user's bubble under a "{verb} for 3m 5s" summary
 * that reads as if it is still working.
 */
async function markInterruptedIfUnanswered(sessionId: string): Promise<void> {
  if (useSessionStore.getState().activeSessionId !== sessionId) return;
  try {
    await useChatStore.getState().fetchSessionState({ sessionId });
  } catch { /* best-effort — judge from what we have */ }
  // The chat on screen is another one now: its rows are not this session's.
  if (!stillOn(sessionId, useSessionStore.getState().activeSessionId)) return;
  // The last row that SPEAKS. A memory recall is persisted right after the
  // user's message, before any reply — counting it as an answer hid every
  // interrupted turn once auto-recall landed.
  // The server kept the question, so a resend is a new message: it shows its own bubble.
  const resend = interruptedResend(
    useChatStore.getState().messages,
    UNSPOKEN_SENDERS,
    useServerStore.getState().selectedAgent,
  );
  if (!resend) return;
  useServerStore.getState().markRunInterrupted(sessionId);
  const ts = new Date();
  useChatStore.getState().addMessage({
    role: 'agent', from: 'system', to: 'user',
    text: INTERRUPTED_TEXT,
    isError: true,
    resend,
    timestamp: ts.toLocaleTimeString(), timestampMs: ts.getTime(), isGenerating: false,
  });
}
