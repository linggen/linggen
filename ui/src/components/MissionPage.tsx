import React, { useState, useEffect, useCallback, useRef, useReducer, useMemo } from 'react';
import { ArrowLeft, Target, Plus, Play, Trash2, Clock, Edit3, Check, X, Eye, ExternalLink, ChevronDown, ChevronRight, Pause, Send, Square } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, ChatMessage, ContentBlock, CronMission, MissionRunEntry, Plan, ProjectInfo, UiSseMessage } from '../types';
import { AgentMessage } from './chat/AgentMessage';
import { getMessagePhase } from './chat/MessagePhase';
import { stripEmbeddedStructuredJson, reconstructContentFromText, shouldHideInternalChatMessage, isPersistedToolOnlyMessage, isStatusLineText } from '../lib/messageUtils';
import { useSseConnection } from '../hooks/useSseConnection';

// ---- Helpers ----------------------------------------------------------------

const formatTimestamp = (ts: number) => {
  if (!ts || ts <= 0) return '-';
  const d = new Date(ts * 1000);
  return d.toLocaleString();
};

const formatShortTime = (ts: number) => {
  if (!ts || ts <= 0) return '-';
  const d = new Date(ts * 1000);
  const now = new Date();
  const isToday = d.toDateString() === now.toDateString();
  if (isToday) return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  return d.toLocaleDateString([], { month: 'short', day: 'numeric' }) + ' ' + d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
};

const describeCron = (schedule: string): string => {
  const parts = schedule.split(/\s+/);
  if (parts.length !== 5) return schedule;
  const [min, hour, dom, mon, dow] = parts;
  if (min === '*' && hour === '*' && dom === '*' && mon === '*' && dow === '*') return 'Every minute';
  if (min.startsWith('*/') && hour === '*' && dom === '*' && mon === '*' && dow === '*') return `Every ${min.slice(2)} min`;
  if (hour.startsWith('*/') && dom === '*' && mon === '*' && dow === '*') return `Every ${hour.slice(2)}h at :${min.padStart(2, '0')}`;
  if (hour === '*' && dom === '*' && mon === '*' && dow === '*') return `Hourly at :${min.padStart(2, '0')}`;
  if (dom === '*' && mon === '*' && dow === '*') return `Daily ${hour}:${min.padStart(2, '0')}`;
  if (dom === '*' && mon === '*' && dow !== '*') {
    const dayNames: Record<string, string> = { '0': 'Sun', '1': 'Mon', '2': 'Tue', '3': 'Wed', '4': 'Thu', '5': 'Fri', '6': 'Sat', '7': 'Sun' };
    if (dow.includes('-')) {
      const [start, end] = dow.split('-');
      return `${dayNames[start] || start}-${dayNames[end] || end} ${hour}:${min.padStart(2, '0')}`;
    }
    const days = dow.split(',').map(d => dayNames[d] || d).join(', ');
    return `${days} ${hour}:${min.padStart(2, '0')}`;
  }
  return schedule;
};

const projectLabel = (path: string | null | undefined, projects: ProjectInfo[]): string | null => {
  if (!path) return null;
  const proj = projects.find(p => p.path === path);
  if (proj) return proj.name || path.split('/').pop() || path;
  return path.split('/').pop() || path;
};

const statusBadgeClass = (run: MissionRunEntry) => {
  if (run.skipped) return 'bg-amber-500/15 text-amber-600';
  if (run.status === 'completed') return 'bg-green-500/15 text-green-600';
  if (run.status === 'failed') return 'bg-red-500/15 text-red-600';
  if (run.status === 'running') return 'bg-blue-500/15 text-blue-600';
  return 'bg-slate-500/15 text-slate-500';
};

// ---- API helpers ------------------------------------------------------------

async function fetchMissions(): Promise<CronMission[]> {
  const resp = await fetch('/api/missions');
  if (!resp.ok) return [];
  const data = await resp.json();
  return Array.isArray(data.missions) ? data.missions : [];
}

async function createMission(name: string | undefined, schedule: string, prompt: string, model?: string, project?: string, permission_tier?: string): Promise<CronMission | null> {
  const resp = await fetch('/api/missions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: name || null, schedule, prompt, model: model || null, project: project || null, permission_tier: permission_tier || 'full' }),
  });
  if (!resp.ok) { throw new Error(await resp.text()); }
  return resp.json();
}

async function updateMission(id: string, updates: Record<string, any>): Promise<CronMission | null> {
  const resp = await fetch(`/api/missions/${encodeURIComponent(id)}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(updates),
  });
  if (!resp.ok) { throw new Error(await resp.text()); }
  return resp.json();
}

async function deleteMission(id: string): Promise<void> {
  await fetch(`/api/missions/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

async function fetchMissionRuns(id: string): Promise<MissionRunEntry[]> {
  const resp = await fetch(`/api/missions/${encodeURIComponent(id)}/runs`);
  if (!resp.ok) return [];
  const data = await resp.json();
  return Array.isArray(data.runs) ? data.runs : [];
}

async function triggerMission(id: string, project?: string): Promise<void> {
  const resp = await fetch(`/api/missions/${encodeURIComponent(id)}/trigger`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ project_root: project || null }),
  });
  if (!resp.ok) { throw new Error(await resp.text()); }
}

async function fetchSessionMessages(projectRoot: string, sessionId: string): Promise<ChatMessage[]> {
  const url = new URL('/api/workspace/state', window.location.origin);
  url.searchParams.append('project_root', projectRoot);
  url.searchParams.append('session_id', sessionId);
  const resp = await fetch(url.toString());
  if (!resp.ok) return [];
  const data = await resp.json();
  if (!data.messages) return [];
  return data.messages
    .filter(([meta, body]: any) => !shouldHideInternalChatMessage(meta.from, body))
    .filter(([_meta, body]: any) => !isPersistedToolOnlyMessage(String(body || '')))
    .flatMap(([meta, body]: any) => {
      const isUser = meta.from === 'user' || meta.from === 'system';
      let bodyStr = String(body || '');
      // Preserve plan JSON messages (rendered as PlanBlock by the UI).
      try {
        const parsed = JSON.parse(bodyStr);
        if (parsed?.type === 'plan' && parsed?.plan) {
          return [{ role: 'agent' as const, from: meta.from, to: meta.to, text: bodyStr, ts: meta.timestamp }];
        }
      } catch { /* not JSON */ }
      if (!isUser) bodyStr = stripEmbeddedStructuredJson(bodyStr);
      if (!isUser && !bodyStr) return [];
      const restored = !isUser ? reconstructContentFromText(bodyStr) : null;
      const isError = !isUser && bodyStr.startsWith('Error:');
      return [{
        role: isUser ? 'user' as const : 'agent' as const,
        from: meta.from, to: meta.to, text: bodyStr,
        timestamp: new Date(meta.ts * 1000).toLocaleTimeString(),
        timestampMs: Number(meta.ts || 0) * 1000,
        ...(restored ? { content: restored.content, toolCount: restored.toolCount } : {}),
        ...(isError ? { isError: true } : {}),
      }];
    });
}

// ---- Lightweight chat reducer for mission sessions -------------------------

type MiniChatAction =
  | { type: 'CLEAR' }
  | { type: 'SYNC_PERSISTED'; messages: ChatMessage[] }
  | { type: 'ADD_USER_MESSAGE'; message: ChatMessage }
  | { type: 'APPEND_TOKEN'; agentId: string; text: string; isThinking: boolean }
  | { type: 'SET_THINKING'; agentId: string }
  | { type: 'ADD_TEXT_SEGMENT'; agentId: string; text: string }
  | { type: 'APPEND_ACTIVITY'; agentId: string; line: string }
  | { type: 'FINALIZE_MESSAGE'; agentId: string; content: string; tsMs: number; isError?: boolean }
  | { type: 'CONTENT_BLOCK_START'; agentId: string; block: ContentBlock }
  | { type: 'CONTENT_BLOCK_UPDATE'; agentId: string; blockId: string; status?: string; summary?: string; isError?: boolean; diffData?: ContentBlock['diffData']; bashOutput?: string[] }
  | { type: 'TURN_COMPLETE'; agentId: string; durationMs?: number; contextTokens?: number };

function findOrCreateGenerating(state: ChatMessage[], agentId: string): [ChatMessage[], number] {
  const idx = state.findIndex(m => m.role === 'agent' && m.from === agentId && m.isGenerating);
  if (idx >= 0) return [state, idx];
  const newMsg: ChatMessage = {
    role: 'agent', from: agentId, to: 'user', text: '',
    timestamp: new Date().toLocaleTimeString(), timestampMs: Date.now(),
    isGenerating: true, content: [],
  };
  return [[...state, newMsg], state.length];
}

function miniChatReducer(state: ChatMessage[], action: MiniChatAction): ChatMessage[] {
  switch (action.type) {
    case 'CLEAR': return [];
    case 'SYNC_PERSISTED': return action.messages;
    case 'ADD_USER_MESSAGE': return [...state, action.message];
    case 'APPEND_TOKEN': {
      const [msgs, idx] = findOrCreateGenerating(state, action.agentId);
      const msg = msgs[idx];
      if (action.isThinking) {
        return [...msgs.slice(0, idx), { ...msg, isThinking: true }, ...msgs.slice(idx + 1)];
      }
      const liveText = (msg.liveText || '') + action.text;
      return [...msgs.slice(0, idx), { ...msg, liveText, isThinking: false }, ...msgs.slice(idx + 1)];
    }
    case 'SET_THINKING': {
      const [msgs, idx] = findOrCreateGenerating(state, action.agentId);
      return [...msgs.slice(0, idx), { ...msgs[idx], isThinking: true }, ...msgs.slice(idx + 1)];
    }
    case 'ADD_TEXT_SEGMENT': {
      const [msgs, idx] = findOrCreateGenerating(state, action.agentId);
      const msg = msgs[idx];
      const text = ((msg.text || '') + '\n' + action.text).trim();
      return [...msgs.slice(0, idx), { ...msg, text, liveText: undefined }, ...msgs.slice(idx + 1)];
    }
    case 'APPEND_ACTIVITY': {
      const [msgs, idx] = findOrCreateGenerating(state, action.agentId);
      const msg = msgs[idx];
      const entries = [...(msg.activityEntries || []), action.line];
      return [...msgs.slice(0, idx), { ...msg, activityEntries: entries }, ...msgs.slice(idx + 1)];
    }
    case 'CONTENT_BLOCK_START': {
      const [msgs, idx] = findOrCreateGenerating(state, action.agentId);
      const msg = msgs[idx];
      const content = [...(msg.content || []), action.block];
      return [...msgs.slice(0, idx), { ...msg, content }, ...msgs.slice(idx + 1)];
    }
    case 'CONTENT_BLOCK_UPDATE': {
      const [msgs, idx] = findOrCreateGenerating(state, action.agentId);
      const msg = msgs[idx];
      const content = (msg.content || []).map(b =>
        b.id === action.blockId ? { ...b, status: action.status || b.status, summary: action.summary ?? b.summary, isError: action.isError, diffData: action.diffData ?? b.diffData, output: action.bashOutput ?? b.output } : b
      );
      return [...msgs.slice(0, idx), { ...msg, content }, ...msgs.slice(idx + 1)];
    }
    case 'FINALIZE_MESSAGE': {
      const idx = state.findIndex(m => m.role === 'agent' && m.from === action.agentId && m.isGenerating);
      if (idx < 0) {
        // No generating message — add as a new finalized message
        return [...state, {
          role: 'agent', from: action.agentId, to: 'user',
          text: action.content, timestamp: new Date(action.tsMs).toLocaleTimeString(),
          timestampMs: action.tsMs, isGenerating: false, isError: action.isError,
        }];
      }
      const msg = state[idx];
      const text = action.content || msg.liveText || msg.text || '';
      return [...state.slice(0, idx), { ...msg, text, liveText: undefined, isGenerating: false, isThinking: false, isError: action.isError }, ...state.slice(idx + 1)];
    }
    case 'TURN_COMPLETE': {
      const idx = state.findIndex(m => m.role === 'agent' && m.from === action.agentId && m.isGenerating);
      if (idx < 0) return state;
      const msg = state[idx];
      const text = msg.liveText || msg.text || '';
      return [...state.slice(0, idx), {
        ...msg, text, liveText: undefined, isGenerating: false, isThinking: false,
        durationMs: action.durationMs, contextTokens: action.contextTokens,
      }, ...state.slice(idx + 1)];
    }
    default: return state;
  }
}

// ---- Mission Chat Panel (interactive) ---------------------------------------

const MISSION_AGENT_ID = 'mission';
/** Agent used for user-initiated messages in mission sessions (conversational, has AskUser). */
const USER_CHAT_AGENT_ID = 'ling';

const MissionChatPanel: React.FC<{
  sessionId: string;
  projectRoot: string;
  mission: CronMission;
  run: MissionRunEntry;
  onOpenSession?: (sessionId: string) => void;
}> = ({ sessionId, projectRoot, mission, run, onOpenSession }) => {
  const [messages, dispatch] = useReducer(miniChatReducer, []);
  const [inputValue, setInputValue] = useState('');
  const [isRunning, setIsRunning] = useState(false);
  const [pendingPlanAgentId, setPendingPlanAgentId] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const shouldAutoScroll = useRef(true);

  const resolvedProject = mission.project || projectRoot;

  // Load persisted messages
  useEffect(() => {
    dispatch({ type: 'CLEAR' });
    setPendingPlanAgentId(null);
    fetchSessionMessages(resolvedProject, sessionId).then(msgs => {
      dispatch({ type: 'SYNC_PERSISTED', messages: msgs });
    });
  }, [sessionId, resolvedProject]);

  // Auto-scroll
  useEffect(() => {
    if (shouldAutoScroll.current && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages]);

  const handleScroll = useCallback(() => {
    if (!scrollRef.current) return;
    const { scrollTop, scrollHeight, clientHeight } = scrollRef.current;
    shouldAutoScroll.current = scrollHeight - scrollTop - clientHeight < 60;
  }, []);

  // SSE connection for live updates
  const handleSseEvent = useCallback((item: UiSseMessage) => {
    const agentId = String(item.agent_id || '');

    switch (item.kind) {
      case 'token': {
        const isThinking = item.data?.thinking === true;
        if (item.phase === 'done') {
          if (isThinking) dispatch({ type: 'SET_THINKING', agentId });
          return;
        }
        dispatch({ type: 'APPEND_TOKEN', agentId, text: String(item.text || ''), isThinking });
        return;
      }
      case 'text_segment': {
        const text = String(item.text || '').trim();
        if (text) dispatch({ type: 'ADD_TEXT_SEGMENT', agentId, text });
        return;
      }
      case 'message': {
        const from = String(item.data?.from || item.agent_id || '');
        let content = String(item.text || '');
        if (!content || shouldHideInternalChatMessage(from, content)) return;
        if (from !== 'user') {
          content = stripEmbeddedStructuredJson(content);
          if (!content) return;
        }
        if (from !== 'user' && isStatusLineText(content)) {
          dispatch({ type: 'APPEND_ACTIVITY', agentId: from, line: content });
          return;
        }
        dispatch({
          type: 'FINALIZE_MESSAGE', agentId: from, content,
          tsMs: item.ts_ms || Date.now(), isError: content.startsWith('Error:'),
        });
        return;
      }
      case 'content_block': {
        const data = item.data || {};
        if (item.phase === 'start' && data.block_type === 'tool_use') {
          dispatch({
            type: 'CONTENT_BLOCK_START', agentId,
            block: { type: 'tool_use', id: data.block_id, tool: data.tool, args: data.args, status: 'running' },
          });
        } else if (item.phase === 'update') {
          dispatch({
            type: 'CONTENT_BLOCK_UPDATE', agentId, blockId: data.block_id,
            status: data.status, summary: data.summary, isError: data.is_error,
            diffData: data.diff_data, bashOutput: data.bash_output,
          });
        }
        return;
      }
      case 'turn_complete': {
        const data = item.data || {};
        dispatch({
          type: 'TURN_COMPLETE', agentId,
          durationMs: typeof data.duration_ms === 'number' ? data.duration_ms : undefined,
          contextTokens: typeof data.context_tokens === 'number' ? data.context_tokens : undefined,
        });
        setIsRunning(false);
        return;
      }
      case 'activity': {
        const status = String(item.data?.status || '');
        if (status === 'idle' || status === 'completed' || status === 'failed' || status === 'cancelled') {
          setIsRunning(false);
        } else if (status === 'working' || status === 'thinking' || status === 'calling_tool' || status === 'model_loading') {
          setIsRunning(true);
        }
        return;
      }
      case 'run': {
        if (item.phase === 'plan_update' && item.data?.plan) {
          const plan = item.data.plan as Plan;
          const match = agentId.match(/^run-(.+?)-\d+/);
          const logicalId = match ? match[1] : agentId;
          const planText = JSON.stringify({ type: 'plan', plan });
          dispatch({ type: 'FINALIZE_MESSAGE', agentId: logicalId, content: planText, tsMs: item.ts_ms || Date.now() });
          if (plan.status === 'planned') {
            setPendingPlanAgentId(logicalId);
          } else {
            setPendingPlanAgentId(null);
          }
        }
        return;
      }
    }
  }, []);

  useSseConnection({ onEvent: handleSseEvent, sessionId });

  // Send message
  const sendMessage = useCallback(async () => {
    const text = inputValue.trim();
    if (!text) return;
    setInputValue('');
    shouldAutoScroll.current = true;

    const now = new Date();
    dispatch({
      type: 'ADD_USER_MESSAGE',
      message: { role: 'user', from: 'user', to: USER_CHAT_AGENT_ID, text, timestamp: now.toLocaleTimeString(), timestampMs: now.getTime() },
    });

    setIsRunning(true);
    try {
      await fetch('/api/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: resolvedProject,
          agent_id: USER_CHAT_AGENT_ID,
          message: text,
          session_id: sessionId,
          mission_id: mission.id,
        }),
      });
    } catch (e) {
      console.error('Failed to send message:', e);
      setIsRunning(false);
    }
  }, [inputValue, resolvedProject, sessionId]);

  // Cancel — either the mission agent or the ling agent could be running.
  const cancelRun = useCallback(async () => {
    try {
      await Promise.all([
        fetch('/api/agent-cancel', {
          method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ project_root: resolvedProject, agent_id: MISSION_AGENT_ID }),
        }),
        fetch('/api/agent-cancel', {
          method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ project_root: resolvedProject, agent_id: USER_CHAT_AGENT_ID }),
        }),
      ]);
    } catch (e) { console.error('Failed to cancel:', e); }
  }, [resolvedProject]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  };

  const approvePlan = useCallback(async (clearContext = false) => {
    if (!pendingPlanAgentId || !resolvedProject) return;
    try {
      await fetch('/api/plan/approve', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: resolvedProject, agent_id: pendingPlanAgentId, session_id: sessionId, clear_context: clearContext }),
      });
      setPendingPlanAgentId(null);
    } catch (e) { console.error('Error approving plan:', e); }
  }, [pendingPlanAgentId, resolvedProject, sessionId]);

  const rejectPlan = useCallback(async () => {
    if (!pendingPlanAgentId || !resolvedProject) return;
    try {
      await fetch('/api/plan/reject', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: resolvedProject, agent_id: pendingPlanAgentId, session_id: sessionId }),
      });
      setPendingPlanAgentId(null);
    } catch (e) { console.error('Error rejecting plan:', e); }
  }, [pendingPlanAgentId, resolvedProject, sessionId]);

  const editPlan = useCallback(async (text: string) => {
    if (!pendingPlanAgentId || !resolvedProject) return;
    try {
      await fetch('/api/plan/edit', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: resolvedProject, agent_id: pendingPlanAgentId, text }),
      });
    } catch (e) { console.error('Error editing plan:', e); }
  }, [pendingPlanAgentId, resolvedProject]);

  const planProps = useMemo(() => ({
    pendingPlanAgentId: pendingPlanAgentId ?? undefined,
    onApprovePlan: approvePlan,
    onRejectPlan: rejectPlan,
    onEditPlan: editPlan,
    inputRef: inputRef as React.RefObject<HTMLTextAreaElement | null>,
  }), [pendingPlanAgentId, approvePlan, rejectPlan, editPlan]);

  return (
    <div className="flex-1 flex flex-col min-h-0">
      {/* Header */}
      <div className="shrink-0 px-5 py-2.5 border-b border-slate-200 dark:border-white/5 flex items-center justify-between">
        <div className="text-[11px] text-slate-500">
          <span className="font-medium text-slate-700 dark:text-slate-300">{mission.name || 'Mission'}</span>
          {' '}&middot; {formatTimestamp(run.triggered_at)}
          {' '}&middot;{' '}
          <span className={cn('font-bold uppercase',
            run.status === 'completed' ? 'text-green-600' : run.status === 'failed' ? 'text-red-600' : run.status === 'running' ? 'text-blue-600' : 'text-slate-500',
          )}>{run.status}</span>
        </div>
        {onOpenSession && (
          <button onClick={() => onOpenSession(sessionId)}
            className="flex items-center gap-1.5 text-[11px] font-medium text-blue-500 hover:text-blue-600 transition-colors">
            <ExternalLink size={11} /> Open in Chat
          </button>
        )}
      </div>

      {/* Messages */}
      <div ref={scrollRef} onScroll={handleScroll} className="flex-1 overflow-y-auto px-5 py-4 space-y-3">
        {messages.map((msg, i) => {
          const isUser = msg.role === 'user';
          const phase = isUser ? undefined : getMessagePhase(msg);
          const messageClass = isUser
            ? 'bg-slate-100 dark:bg-white/10 text-slate-900 dark:text-slate-100 rounded-md px-2.5 py-1.5'
            : phase === 'thinking' ? ''
              : msg.isThinking && !msg.isGenerating ? 'text-slate-500 dark:text-slate-400 italic opacity-60'
                : 'text-slate-800 dark:text-slate-200';
          const agentLabel = !isUser && msg.from && msg.from !== 'user' ? msg.from : null;
          return (
            <div key={`${msg.timestampMs}-${i}`} className={cn('w-full flex', isUser ? 'justify-end' : 'justify-start')}>
              <div className={cn(isUser ? 'max-w-[96%]' : 'max-w-full', 'text-[13px] leading-relaxed', messageClass)}>
                {agentLabel && (
                  <div className="text-[10px] font-semibold uppercase tracking-wide text-slate-400 dark:text-slate-500 mb-0.5">
                    {agentLabel}
                  </div>
                )}
                {isUser ? <span>{msg.text}</span> : (
                  <AgentMessage msg={msg} isExpanded={false} onToggle={() => {}} planProps={planProps} />
                )}
              </div>
            </div>
          );
        })}
      </div>

      {/* Input */}
      <div className="shrink-0 border-t border-slate-200 dark:border-white/5 px-4 py-3">
        <div className="flex items-end gap-2">
          <textarea
            ref={inputRef}
            value={inputValue}
            onChange={e => setInputValue(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Continue the conversation..."
            rows={1}
            className="flex-1 resize-none px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30 max-h-32"
          />
          {isRunning ? (
            <button onClick={cancelRun}
              className="p-2 rounded-lg bg-red-500 text-white hover:bg-red-600 transition-colors shrink-0" title="Stop">
              <Square size={16} />
            </button>
          ) : (
            <button onClick={sendMessage} disabled={!inputValue.trim()}
              className="p-2 rounded-lg bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-40 transition-colors shrink-0" title="Send">
              <Send size={16} />
            </button>
          )}
        </div>
      </div>
    </div>
  );
};

// ---- Left Sidebar: Mission Nav with expandable runs -------------------------

/** Selected item in the sidebar: either a run's session or the mission editor. */
type SidebarSelection =
  | { type: 'run'; missionId: string; run: MissionRunEntry }
  | { type: 'editor'; missionId: string }
  | { type: 'agent-viewer' }
  | null;

const MissionNav: React.FC<{
  missions: CronMission[];
  projects: ProjectInfo[];
  runsMap: Record<string, MissionRunEntry[]>;
  expandedMissions: Set<string>;
  selection: SidebarSelection;
  onToggleExpand: (id: string) => void;
  onSelectRun: (mission: CronMission, run: MissionRunEntry) => void;
  onToggleEnabled: (id: string, enabled: boolean) => void;
  onEdit: (m: CronMission) => void;
  onDelete: (id: string) => void;
  onTrigger: (m: CronMission) => void;
  onCreate: () => void;
}> = ({ missions, projects, runsMap, expandedMissions, selection, onToggleExpand, onSelectRun, onToggleEnabled: _onToggleEnabled, onEdit, onDelete, onTrigger, onCreate }) => {
  const enabledCount = missions.filter(m => m.enabled).length;
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  return (
    <div className="flex-1 flex flex-col min-h-0">
      {/* Header */}
      <div className="p-3 border-b border-slate-200 dark:border-white/5 flex items-center gap-2">
        <button
          onClick={onCreate}
          className="flex-1 flex items-center justify-center gap-1.5 px-3 py-1.5 text-[11px] font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 transition-colors"
        >
          <Plus size={13} /> New Mission
        </button>
        {enabledCount > 0 && (
          <span className="text-[9px] font-bold px-1.5 py-0.5 rounded-full bg-green-500/15 text-green-600 dark:text-green-400 shrink-0">
            {enabledCount} active
          </span>
        )}
      </div>

      {/* Mission list (scrollable) */}
      <div className="flex-1 overflow-y-auto p-2 space-y-1">
        {missions.length === 0 && (
          <div className="p-4 text-xs text-slate-500 italic text-center">
            No missions yet. Create one to start.
          </div>
        )}

        {missions.map(mission => {
          const isExpanded = expandedMissions.has(mission.id);
          const runs = runsMap[mission.id] || [];
          const sortedRuns = [...runs].reverse();
          const projLabel = projectLabel(mission.project, projects);

          return (
            <div key={mission.id} className="rounded-lg">
              {/* Mission header */}
              <div className="relative group">
                <button
                  onClick={() => onToggleExpand(mission.id)}
                  className={cn(
                    'w-full text-left px-2.5 py-2 rounded-lg transition-colors',
                    (selection?.type === 'run' && selection.missionId === mission.id) ||
                    (selection?.type === 'editor' && selection.missionId === mission.id)
                      ? 'bg-blue-50 dark:bg-blue-500/10'
                      : 'hover:bg-slate-50 dark:hover:bg-white/5',
                  )}
                >
                  <div className="flex items-center gap-1.5">
                    {isExpanded
                      ? <ChevronDown size={13} className="text-slate-400 shrink-0" />
                      : <ChevronRight size={13} className="text-slate-400 shrink-0" />
                    }
                    <span className={cn(
                      'w-2 h-2 rounded-full shrink-0',
                      mission.enabled ? 'bg-green-500' : 'bg-slate-300 dark:bg-slate-600',
                    )} />
                    <span className="text-[11px] font-bold text-slate-800 dark:text-slate-200 truncate">
                      {mission.name || 'Untitled Mission'}
                    </span>
                  </div>
                  <div className="ml-5 text-[10px] text-slate-400 truncate mt-0.5">
                    {describeCron(mission.schedule)}
                    {projLabel && <> &middot; {projLabel}</>}
                  </div>
                  {!isExpanded && runs.length > 0 && (
                    <div className="ml-5 text-[10px] text-slate-400 mt-0.5">
                      {runs.length} run{runs.length !== 1 ? 's' : ''}
                    </div>
                  )}
                </button>

                {/* Action buttons on hover */}
                <div className="absolute right-1.5 top-1.5 flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-all">
                  <button
                    onClick={(e) => { e.stopPropagation(); onTrigger(mission); }}
                    className="p-1 rounded hover:bg-green-100 dark:hover:bg-green-500/10 text-slate-400 hover:text-green-600"
                    title="Run now"
                  >
                    <Play size={11} />
                  </button>
                  <button
                    onClick={(e) => { e.stopPropagation(); onEdit(mission); }}
                    className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400 hover:text-slate-600"
                    title="Edit"
                  >
                    <Edit3 size={11} />
                  </button>
                  {confirmDeleteId === mission.id ? (
                    <>
                      <button
                        onClick={(e) => { e.stopPropagation(); onDelete(mission.id); setConfirmDeleteId(null); }}
                        className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-500/10 text-red-500"
                        title="Confirm delete"
                      >
                        <Check size={11} />
                      </button>
                      <button
                        onClick={(e) => { e.stopPropagation(); setConfirmDeleteId(null); }}
                        className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400"
                        title="Cancel"
                      >
                        <X size={11} />
                      </button>
                    </>
                  ) : (
                    <button
                      onClick={(e) => { e.stopPropagation(); setConfirmDeleteId(mission.id); }}
                      className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-500/10 text-slate-400 hover:text-red-500"
                      title="Delete"
                    >
                      <Trash2 size={11} />
                    </button>
                  )}
                </div>
              </div>

              {/* Expanded: run sessions list */}
              {isExpanded && (
                <div className="ml-3 mt-0.5 space-y-0.5">
                  {sortedRuns.length === 0 ? (
                    <div className="px-2.5 py-2 text-[10px] text-slate-400 italic">
                      No runs yet
                    </div>
                  ) : sortedRuns.map((run, i) => {
                    const isActive = selection?.type === 'run' && selection.missionId === mission.id && selection.run.run_id === run.run_id;
                    return (
                      <button
                        key={`${run.run_id}-${i}`}
                        onClick={() => onSelectRun(mission, run)}
                        className={cn(
                          'w-full text-left px-2.5 py-1.5 rounded-lg transition-colors text-[11px]',
                          isActive
                            ? 'bg-blue-100/80 dark:bg-blue-500/15 border-l-2 border-blue-500'
                            : 'hover:bg-slate-50 dark:hover:bg-white/5',
                        )}
                      >
                        <div className="flex items-center gap-2">
                          <span className="text-slate-600 dark:text-slate-300 font-medium">
                            {formatShortTime(run.triggered_at)}
                          </span>
                          <span className={cn('text-[9px] font-bold px-1 py-0 rounded uppercase tracking-wide', statusBadgeClass(run))}>
                            {run.skipped ? 'skip' : run.status === 'completed' ? 'ok' : run.status}
                          </span>
                        </div>
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
};

// ---- Mission Editor ---------------------------------------------------------

const CRON_PRESETS = [
  { label: 'Every 30 min', value: '*/30 * * * *' },
  { label: 'Every hour', value: '0 * * * *' },
  { label: 'Every 2 hours', value: '0 */2 * * *' },
  { label: 'Daily at 9am', value: '0 9 * * *' },
  { label: 'Weekdays 9am', value: '0 9 * * 1-5' },
  { label: 'Weekly Sunday', value: '0 0 * * 0' },
];

export const PERMISSION_TIERS = [
  { value: 'readonly', label: 'Read-only', desc: 'Analyze and report only. No file changes or commands.', color: 'green' },
  { value: 'standard', label: 'Standard', desc: 'Read + edit files, run build/test commands. Requires a project.', color: 'blue' },
  { value: 'full', label: 'Full access', desc: 'All tools, no restrictions. Use with caution.', color: 'amber' },
] as const;

export const MissionEditor: React.FC<{
  editing: CronMission | null;
  projects: ProjectInfo[];
  onSave: (mission: CronMission) => void;
  onCancel: () => void;
  onViewAgent: () => void;
}> = ({ editing, projects, onSave, onCancel, onViewAgent }) => {
  const [name, setName] = useState(editing?.name || '');
  const [schedule, setSchedule] = useState(editing?.schedule || '*/30 * * * *');
  const [prompt, setPrompt] = useState(editing?.prompt || '');
  const [model, setModel] = useState(editing?.model || '');
  const [selectedProject, setSelectedProject] = useState(editing?.project || '');
  const [permissionTier, setPermissionTier] = useState(editing?.permission_tier || 'full');
  const [models, setModels] = useState<{ id: string; model: string; provider: string }[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    fetch('/api/config').then(r => r.ok ? r.json() : null).then(data => { if (data?.models) setModels(data.models); }).catch(() => {});
  }, []);

  // Standard tier requires a project
  const tierError = permissionTier === 'standard' && !selectedProject
    ? 'Standard tier requires a project to scope file edits.'
    : null;

  const handleSave = async () => {
    if (!schedule.trim() || !prompt.trim()) { setError('Schedule and prompt are required'); return; }
    if (tierError) { setError(tierError); return; }
    setSaving(true); setError(null);
    try {
      const result = editing
        ? await updateMission(editing.id, { name: name || null, schedule, prompt, model: model || null, project: selectedProject || null, permission_tier: permissionTier })
        : await createMission(name || undefined, schedule, prompt, model || undefined, selectedProject || undefined, permissionTier);
      if (result) onSave(result);
    } catch (e: any) { setError(e.message || 'Failed to save mission'); }
    setSaving(false);
  };

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="max-w-2xl mx-auto space-y-4">
        <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
          {editing ? 'Edit Mission' : 'New Mission'}
        </h2>

        {error && <div className="bg-red-500/10 border border-red-500/20 rounded-lg p-3 text-xs text-red-600 dark:text-red-400">{error}</div>}

        <div>
          <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Name</label>
          <input type="text" value={name} onChange={e => setName(e.target.value)} placeholder="e.g. Daily code review"
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Cron Schedule</label>
          <input type="text" value={schedule} onChange={e => setSchedule(e.target.value)} placeholder="*/30 * * * *"
            className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
          <div className="flex flex-wrap gap-1.5 mt-2">
            {CRON_PRESETS.map(p => (
              <button key={p.value} onClick={() => setSchedule(p.value)} className={cn(
                'text-[10px] px-2 py-0.5 rounded-full border transition-colors',
                schedule === p.value ? 'border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-400' : 'border-slate-200 dark:border-white/10 text-slate-500 hover:bg-slate-50 dark:hover:bg-white/5',
              )}>{p.label}</button>
            ))}
          </div>
          <div className="text-[10px] text-slate-400 mt-1.5">{describeCron(schedule)}</div>
        </div>

        <div>
          <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Agent</label>
          <div className="flex items-center gap-2">
            <div className="flex-1 px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-white/[0.03] text-slate-500">
              <span className="font-semibold text-purple-600 dark:text-purple-400">mission</span>
              <span className="text-slate-400 ml-2">— Autonomous (no human interaction)</span>
            </div>
            <button onClick={onViewAgent} className="flex items-center gap-1 px-2.5 py-2 text-xs font-medium rounded-lg border border-slate-200 dark:border-white/10 text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5 transition-colors shrink-0" title="View mission.md">
              <Eye size={13} /> View
            </button>
          </div>
        </div>

        <div>
          <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Permissions</label>
          <div className="space-y-2">
            {PERMISSION_TIERS.map(tier => {
              const selected = permissionTier === tier.value;
              const disabled = tier.value === 'standard' && !selectedProject;
              const colorMap = {
                green: selected ? 'border-green-500/40 bg-green-500/10' : '',
                blue: selected ? 'border-blue-500/40 bg-blue-500/10' : '',
                amber: selected ? 'border-amber-500/40 bg-amber-500/10' : '',
              };
              const dotMap = {
                green: 'bg-green-500',
                blue: 'bg-blue-500',
                amber: 'bg-amber-500',
              };
              return (
                <button
                  key={tier.value}
                  onClick={() => !disabled && setPermissionTier(tier.value)}
                  disabled={disabled}
                  className={cn(
                    'w-full flex items-start gap-3 px-3 py-2.5 rounded-lg border text-left transition-colors',
                    selected
                      ? colorMap[tier.color]
                      : 'border-slate-200 dark:border-white/10 hover:bg-slate-50 dark:hover:bg-white/5',
                    disabled && 'opacity-40 cursor-not-allowed',
                  )}
                >
                  <div className={cn('w-3 h-3 rounded-full mt-0.5 shrink-0 border-2', selected ? dotMap[tier.color] + ' border-transparent' : 'border-slate-300 dark:border-white/20')} />
                  <div className="min-w-0">
                    <div className="text-xs font-semibold text-slate-700 dark:text-slate-200">{tier.label}</div>
                    <div className="text-[10px] text-slate-500 dark:text-slate-400 mt-0.5">{tier.desc}</div>
                    {tier.value === 'standard' && !selectedProject && (
                      <div className="text-[10px] text-amber-600 dark:text-amber-400 mt-0.5">Select a project below to enable this tier</div>
                    )}
                    {tier.value === 'readonly' && selected && (
                      <div className="text-[10px] text-slate-400 mt-0.5">Tools: Read, Glob, Grep, WebSearch, WebFetch, Task</div>
                    )}
                    {tier.value === 'standard' && selected && (
                      <div className="text-[10px] text-slate-400 mt-0.5">Tools: Read, Write, Edit, Glob, Grep, Bash (build/test only), WebSearch, WebFetch, Task, Skill</div>
                    )}
                    {tier.value === 'full' && selected && (
                      <div className="text-[10px] text-slate-400 mt-0.5">All tools including unrestricted Bash</div>
                    )}
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        <div>
          <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Model <span className="text-slate-400">(optional)</span></label>
          <select value={model} onChange={e => setModel(e.target.value)}
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30">
            <option value="">Default (inherit from agent)</option>
            {models.map(m => <option key={m.id} value={m.id}>{m.id} — {m.provider}/{m.model}</option>)}
          </select>
        </div>

        <div>
          <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Project {permissionTier === 'standard' && <span className="text-amber-600 dark:text-amber-400">(required for Standard tier)</span>}
            {permissionTier !== 'standard' && <span className="text-slate-400">(optional)</span>}
          </label>
          <select value={selectedProject} onChange={e => setSelectedProject(e.target.value)}
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30">
            <option value="">No project (global)</option>
            {projects.map(p => <option key={p.path} value={p.path}>{p.name || p.path.split('/').pop()}</option>)}
          </select>
        </div>

        <div>
          <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Prompt</label>
          <textarea value={prompt} onChange={e => setPrompt(e.target.value)} placeholder="The instruction to send to the agent on each trigger..." rows={6}
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 resize-y focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div className="flex items-center gap-3 pt-2">
          <button onClick={handleSave} disabled={saving || !prompt.trim() || !!tierError}
            className="px-4 py-2 text-sm font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-40 disabled:cursor-not-allowed transition-colors">
            {saving ? 'Saving...' : editing ? 'Update Mission' : 'Create Mission'}
          </button>
          <button onClick={onCancel}
            className="px-4 py-2 text-sm font-semibold rounded-lg border border-slate-200 dark:border-white/10 text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5 transition-colors">
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
};

// ---- Agent Viewer (readonly) ------------------------------------------------

const AgentViewer: React.FC<{ onBack: () => void; projectRoot: string }> = ({ onBack, projectRoot }) => {
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const tryFetch = async () => {
      for (const path of ['agents/mission.md', '~/.linggen/agents/mission.md']) {
        const url = new URL('/api/agent-file', window.location.origin);
        url.searchParams.append('project_root', projectRoot);
        url.searchParams.append('path', path);
        const resp = await fetch(url.toString());
        if (resp.ok) { const data = await resp.json(); if (data.content) return data.content; }
      }
      return null;
    };
    tryFetch().then(setContent).catch(() => setContent(null)).finally(() => setLoading(false));
  }, [projectRoot]);

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="max-w-2xl mx-auto space-y-4">
        <div className="flex items-center gap-3">
          <button onClick={onBack} className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500"><ArrowLeft size={14} /></button>
          <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
            Mission Agent <span className="text-[10px] text-slate-400 font-normal ml-1">agents/mission.md</span>
          </h2>
        </div>
        {loading ? <div className="text-center py-16 text-sm text-slate-400">Loading...</div>
          : content ? <pre className="text-xs font-mono whitespace-pre-wrap bg-slate-50 dark:bg-white/[0.03] border border-slate-200 dark:border-white/10 rounded-lg p-4 overflow-x-auto text-slate-700 dark:text-slate-300">{content}</pre>
          : <div className="text-center py-16 text-sm text-slate-400">Could not load mission.md</div>
        }
      </div>
    </div>
  );
};

// ---- Right Panel: Session chat or empty state -------------------------------

const RightPanel: React.FC<{
  selection: SidebarSelection;
  projectRoot: string;
  missions: CronMission[];
  onOpenSession?: (sessionId: string) => void;
}> = ({ selection, projectRoot, missions, onOpenSession }) => {
  const selectedMission = selection?.type === 'run' ? missions.find(m => m.id === selection.missionId) : null;
  const _selectedRun = selection?.type === 'run' ? selection.run : null;

  if (!selection) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center text-slate-400">
        <Target size={32} className="mb-3 opacity-30" />
        <p className="text-sm">Select a mission run to view its session</p>
        <p className="text-[11px] mt-1 text-slate-400">Or create a new mission to get started</p>
      </div>
    );
  }

  if (selection.type === 'run') {
    const run = selection.run;

    if (run.skipped) {
      return (
        <div className="flex-1 flex flex-col items-center justify-center text-slate-400">
          <Pause size={28} className="mb-2 opacity-40 text-amber-500" />
          <p className="text-sm">Run was skipped</p>
          <p className="text-[11px] mt-1">Agent was busy when this trigger fired</p>
        </div>
      );
    }

    if (!run.session_id) {
      return (
        <div className="flex-1 flex flex-col items-center justify-center text-slate-400">
          <Clock size={28} className="mb-2 opacity-40" />
          <p className="text-sm">No session recorded</p>
        </div>
      );
    }

    return (
      <MissionChatPanel
        sessionId={run.session_id}
        projectRoot={projectRoot}
        mission={selectedMission || { id: '', schedule: '', prompt: '', enabled: false, created_at: 0 } as CronMission}
        run={run}
        onOpenSession={onOpenSession}
      />
    );
  }

  return null;
};

// ---- Main Page --------------------------------------------------------------

export const MissionPage: React.FC<{
  onBack: () => void;
  projectRoot: string;
  agents: AgentInfo[];
  embedded?: boolean;
  onOpenSession?: (sessionId: string) => void;
}> = ({ onBack, projectRoot, agents: _agents, embedded, onOpenSession }) => {
  const [missions, setMissions] = useState<CronMission[]>([]);
  const [loading, setLoading] = useState(true);
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [expandedMissions, setExpandedMissions] = useState<Set<string>>(new Set());
  const [runsMap, setRunsMap] = useState<Record<string, MissionRunEntry[]>>({});
  const [selection, setSelection] = useState<SidebarSelection>(null);
  const [rightView, setRightView] = useState<'session' | 'editor' | 'agent-viewer'>('session');
  const [editingMission, setEditingMission] = useState<CronMission | null>(null);

  // Fetch projects
  useEffect(() => {
    fetch('/api/projects').then(r => r.ok ? r.json() : []).then((data: ProjectInfo[]) => setProjects(data)).catch(() => {});
  }, []);

  const loadMissions = useCallback(async () => {
    const data = await fetchMissions();
    setMissions(data);
    setLoading(false);
  }, []);

  useEffect(() => { setLoading(true); loadMissions(); }, [loadMissions]);

  // Fetch runs when a mission is expanded
  const loadRuns = useCallback(async (missionId: string) => {
    const runs = await fetchMissionRuns(missionId);
    setRunsMap(prev => ({ ...prev, [missionId]: runs }));
  }, []);

  const handleToggleExpand = useCallback((id: string) => {
    setExpandedMissions(prev => {
      const next = new Set(prev);
      if (next.has(id)) { next.delete(id); } else { next.add(id); loadRuns(id); }
      return next;
    });
  }, [loadRuns]);

  const handleSelectRun = useCallback((mission: CronMission, run: MissionRunEntry) => {
    setSelection({ type: 'run', missionId: mission.id, run });
    setRightView('session');
  }, []);

  const handleToggle = async (id: string, enabled: boolean) => {
    try { await updateMission(id, { enabled }); await loadMissions(); } catch (e) { console.error('Failed to toggle mission:', e); }
  };

  const handleDelete = async (id: string) => {
    try { await deleteMission(id); await loadMissions(); if (selection && 'missionId' in selection && selection.missionId === id) setSelection(null); } catch (e) { console.error('Failed to delete mission:', e); }
  };

  const handleEdit = (m: CronMission) => {
    setEditingMission(m);
    setSelection({ type: 'editor', missionId: m.id });
    setRightView('editor');
  };

  const handleTrigger = async (m: CronMission) => {
    try {
      await triggerMission(m.id, m.project || undefined);
      // Refresh runs after a short delay to pick up the new run
      setTimeout(() => loadRuns(m.id), 2000);
    } catch (e: any) { console.error('Failed to trigger mission:', e); }
  };

  const handleCreate = () => {
    setEditingMission(null);
    setSelection(null);
    setRightView('editor');
  };

  const handleSave = async (_mission: CronMission) => {
    setEditingMission(null);
    setRightView('session');
    setSelection(null);
    await loadMissions();
  };

  const handleCancel = () => {
    setEditingMission(null);
    setRightView('session');
  };

  const handleViewAgent = () => {
    setSelection({ type: 'agent-viewer' });
    setRightView('agent-viewer');
  };

  const enabledCount = missions.filter(m => m.enabled).length;

  // Full-page views (editor, agent viewer) hide the sidebar
  const isFullPageView = rightView === 'editor' || rightView === 'agent-viewer';

  const mainContent = (
    <div className="flex-1 flex overflow-hidden">
      {/* Left sidebar — mission nav (hidden during editor/agent-viewer) */}
      {!isFullPageView && (
        <div className="w-72 border-r border-slate-200 dark:border-white/5 flex flex-col bg-white dark:bg-[#0f0f0f] h-full">
          <MissionNav
            missions={missions}
            projects={projects}
            runsMap={runsMap}
            expandedMissions={expandedMissions}
            selection={selection}
            onToggleExpand={handleToggleExpand}
            onSelectRun={handleSelectRun}
            onToggleEnabled={handleToggle}
            onEdit={handleEdit}
            onDelete={handleDelete}
            onTrigger={handleTrigger}
            onCreate={handleCreate}
          />
        </div>
      )}

      {/* Right panel — session content or full-page editor */}
      <main className="flex-1 flex flex-col overflow-hidden bg-slate-100/40 dark:bg-[#0a0a0a] min-h-0">
        {loading ? (
          <div className="flex-1 flex items-center justify-center text-sm text-slate-400">Loading...</div>
        ) : rightView === 'editor' ? (
          <MissionEditor
            editing={editingMission}
            projects={projects}
            onSave={handleSave}
            onCancel={handleCancel}
            onViewAgent={handleViewAgent}
          />
        ) : rightView === 'agent-viewer' ? (
          <AgentViewer
            onBack={() => { setRightView(editingMission ? 'editor' : 'session'); }}
            projectRoot={projectRoot}
          />
        ) : (
          <RightPanel
            selection={selection}
            projectRoot={projectRoot}
            missions={missions}
            onOpenSession={onOpenSession}
          />
        )}
      </main>
    </div>
  );

  if (embedded) {
    return <div className="flex flex-col h-full">{mainContent}</div>;
  }

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200">
      <header className="flex items-center gap-4 px-6 py-3 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md shrink-0">
        <button onClick={onBack} className="p-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500 transition-colors">
          <ArrowLeft size={16} />
        </button>
        <div className="flex items-center gap-2">
          <Target size={18} className={enabledCount > 0 ? 'text-green-500' : 'text-slate-400'} />
          <h1 className="text-lg font-bold tracking-tight">Missions</h1>
        </div>
        {enabledCount > 0 && (
          <span className="text-[10px] font-bold uppercase tracking-wide px-2 py-0.5 rounded-full bg-green-500/15 text-green-600 dark:text-green-400">
            {enabledCount} active
          </span>
        )}
      </header>
      {mainContent}
    </div>
  );
};
