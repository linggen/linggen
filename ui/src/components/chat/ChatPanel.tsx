import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Sparkles, ArrowDown, Copy, FileText, Eraser, Plus } from 'lucide-react';
import 'highlight.js/styles/github.css';
import { cn } from '../../lib/cn';
import { useSessionStore } from '../../stores/sessionStore';
import { useServerStore } from '../../stores/serverStore';
import { useUserStore } from '../../stores/userStore';
import { AskUserCard } from '../AskUserCard';
import { ToolPermissionCard } from '../ToolPermissionCard';
import type {
  AgentInfo,
  ChatMessage,
  QueuedChatItem,
  SkillInfo,
  SubagentInfo,
} from '../../types';
import { normalizeAgentKey } from './utils/message';
import { ChatMessageList, ChatMessageRow } from './ChatMessageList';
import { RunStatusLine } from './RunStatusLine';
import { useChatFilters } from './useChatFilters';
import { useFloatingUserMessage } from './useFloatingUserMessage';
import { useMessageWindow } from './useMessageWindow';
import { useRunSpinner } from './useRunSpinner';
import { useSubagentPane } from './useSubagentPane';
import { ChatInput } from './ChatInput';
import { SubagentDrawer } from './SubagentDrawer';
import { SubagentPane } from './SubagentPane';
import { statusBadgeClass } from './MessageHelpers';
import { SessionModelSelector, SessionModeSelector, SessionStats } from './SessionSelectors';
import { useChatActions } from '../../hooks/useChatActions';
import { useChatStore } from '../../stores/chatStore';
import { sessions as sessionsApi } from '../../lib/api';
import { sessionApi } from '../../lib/endpoints';
import { postToParent } from '../../lib/parentFrame';

/**
 * Debug action buttons shown inside the expanded session header.
 * Hidden by default — only visible when the user clicks the session bar open.
 */
const ChatDebugActions: React.FC<{ projectRoot?: string | null; sessionId?: string | null }> = ({ projectRoot, sessionId }) => {
  const [copyStatus, setCopyStatus] = useState<'idle' | 'copied' | 'error'>('idle');
  const [spStatus, setSpStatus] = useState<'idle' | 'copied' | 'error'>('idle');
  const { copyChat, clearChat } = useChatActions(() => {}, {}, projectRoot);

  const handleCopyChat = useCallback(async () => {
    try { await copyChat(); setCopyStatus('copied'); }
    catch { setCopyStatus('error'); }
    setTimeout(() => setCopyStatus('idle'), 1500);
  }, [copyChat]);

  const handleCopySystemPrompt = useCallback(async () => {
    const root = projectRoot || useSessionStore.getState().selectedProjectRoot || '';
    const agentId = useServerStore.getState().selectedAgent;
    const sid = sessionId || useSessionStore.getState().activeSessionId;
    try {
      const payload = await sessionApi.systemPrompt(root, agentId, sid);
      const promptText = payload.system_prompt || '';
      const tools = Array.isArray(payload.tools) ? payload.tools : [];
      if (!promptText && tools.length === 0) throw new Error('empty payload');
      // Combine the system prompt and the tool schemas so the copy shows the
      // full model-facing surface. Tool schemas travel via the native API
      // `tools` parameter, not the prompt text, but they are part of what the
      // model actually receives.
      const toolsSection = tools.length > 0
        ? `\n\n--- TOOLS (passed via native function calling) ---\n${JSON.stringify(tools, null, 2)}\n--- END TOOLS ---\n`
        : '';
      const text = `${promptText}${toolsSection}`;
      try { await navigator.clipboard.writeText(text); }
      catch (clipErr) {
        // Browser may block clipboard outside a user gesture (e.g. after the
        // await round-trip) — fall back to the textarea+execCommand trick.
        const ta = document.createElement('textarea');
        ta.value = text; ta.style.position = 'fixed'; ta.style.opacity = '0';
        document.body.appendChild(ta); ta.select();
        const ok = document.execCommand('copy');
        document.body.removeChild(ta);
        if (!ok) throw new Error(`clipboard fallback failed: ${(clipErr as Error)?.message ?? 'unknown'}`);
      }
      setSpStatus('copied');
    } catch (err) {
      console.error('[copy-system-prompt] failed:', err);
      setSpStatus('error');
    }
    setTimeout(() => setSpStatus('idle'), 1500);
  }, [projectRoot, sessionId]);

  const btnClass = (status: 'idle' | 'copied' | 'error') => cn(
    'px-1.5 py-1 rounded transition-colors text-slate-500 dark:text-slate-400 shrink-0 flex items-center gap-1 text-[11px] font-medium',
    status === 'copied' ? 'bg-green-500/10 text-green-600'
      : status === 'error' ? 'bg-red-500/10 text-red-500'
      : 'hover:bg-slate-100 dark:hover:bg-white/5',
  );

  const labelFor = (status: 'idle' | 'copied' | 'error', idle: string) =>
    status === 'copied' ? 'Copied' : status === 'error' ? 'Failed' : idle;

  // A fresh session in place — the way an app page (its toolbar hidden)
  // gets a new chat, e.g. to bind a skill's rules changed since the last
  // one. A skill session stays bound to its skill; the parent page is told
  // the way a first send tells it, so its bridge follows the new id.
  const [newStatus, setNewStatus] = useState<'idle' | 'copied' | 'error'>('idle');
  const handleNewChat = useCallback(async () => {
    const ss = useSessionStore.getState();
    const skill = ss.isSkillSession && ss.activeSkillName ? ss.activeSkillName : undefined;
    const now = new Date();
    const title = skill
      ? `${skill} session`
      : `Chat ${now.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}, ${now.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })}`;
    try {
      const data = await sessionsApi.create({ title, ...(skill ? { skill } : {}) });
      const created = { id: data.id, repo_path: '', title, created_at: Math.floor(Date.now() / 1000), ...(skill ? { skill } : {}) };
      useSessionStore.setState((s) => ({ activeSessionId: data.id, allSessions: [created, ...s.allSessions] }));
      const cs = useChatStore.getState();
      cs.setActiveSession(data.id);
      cs.fetchSessionState();
      postToParent({ type: 'linggen-skill-event', event: 'session_created', payload: { sessionId: data.id } });
      setNewStatus('copied');
    } catch (err) {
      console.error('[new-chat] failed:', err);
      setNewStatus('error');
    }
    setTimeout(() => setNewStatus('idle'), 1500);
  }, []);

  return (
    <div className="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
      <button onClick={handleNewChat} className={btnClass(newStatus)} title="Start a new chat session here">
        <Plus size={12} />
        <span>{newStatus === 'copied' ? 'New' : labelFor(newStatus, 'New chat')}</span>
      </button>
      <button onClick={handleCopyChat} className={btnClass(copyStatus)} title="Copy chat transcript to clipboard">
        <Copy size={12} />
        <span>{labelFor(copyStatus, 'Chat')}</span>
      </button>
      <button onClick={handleCopySystemPrompt} className={btnClass(spStatus)} title="Copy system prompt + tool schemas to clipboard">
        <FileText size={12} />
        <span>{labelFor(spStatus, 'System Prompt')}</span>
      </button>
      <button onClick={clearChat}
        className="px-1.5 py-1 rounded text-slate-500 dark:text-slate-400 hover:bg-red-500/10 hover:text-red-500 transition-colors shrink-0 flex items-center gap-1 text-[11px] font-medium"
        title="Clear chat messages">
        <Eraser size={12} />
        <span>Clear</span>
      </button>
    </div>
  );
};

export const ChatPanel: React.FC<{
  chatMessages: ChatMessage[];
  queuedMessages: QueuedChatItem[];
  chatEndRef: React.RefObject<HTMLDivElement | null>;
  projectRoot?: string | null;
  sessionId?: string | null;
  selectedAgent: string;
  setSelectedAgent: (value: string) => void;
  skills: SkillInfo[];
  agents: AgentInfo[];
  mainAgents: AgentInfo[];
  subagents: SubagentInfo[];
  runningMainRunIds?: Record<string, string>;
  runningRunIdBySession?: Record<string, string>;
  cancellingRunIds?: Record<string, boolean>;
  onCancelRun?: (runId: string) => void | Promise<void>;
  onSendMessage: (message: string, targetAgent?: string, images?: string[]) => void;
  activePlan?: import('../../types').Plan | null;
  pendingPlan?: import('../../types').Plan | null;
  pendingPlanAgentId?: string | null;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  onApprovePlan?: () => void;
  onRejectPlan?: () => void;
  onEditPlan?: (text: string) => void;
  onResend?: (failed: import('../../types').ChatMessage) => void;
  pendingAskUser?: import('../../types').PendingAskUser | null;
  onRespondToAskUser?: (questionId: string, answers: import('../../types').AskUserAnswer[]) => void;
  onCancelAgentRun?: (runId: string) => void | Promise<void>;
  isRunning?: boolean;
  verboseMode?: boolean;
  overlay?: string | null;
  onDismissOverlay?: () => void;
  modelPickerOpen?: boolean;
  models?: import('../../types').ModelInfo[];
  defaultModels?: string[];
  onSwitchModel?: (modelId: string) => void;
  tokensPerSec?: number;
  mobile?: boolean;
  scrollToBottom?: () => void;
  showScrollButton?: boolean;
  /** Render the dedicated SubagentPane on the right (or stacked on
   *  narrow screens). Default true at the top-level Linggen UI; iframe
   *  consumers should pass `false` and let the inline SubagentTreeView
   *  inside each parent message handle subagent visibility instead. */
  showSubagentPane?: boolean;
}> = ({
  chatMessages,
  queuedMessages,
  chatEndRef,
  projectRoot,
  sessionId,
  selectedAgent,
  setSelectedAgent,
  skills,
  agents,
  mainAgents,
  subagents,
  runningMainRunIds,
  runningRunIdBySession,
  cancellingRunIds,
  onCancelRun,
  onSendMessage,
  activePlan,
  pendingPlan: _pendingPlan,
  pendingPlanAgentId,
  agentContext,
  onApprovePlan,
  onRejectPlan,
  onEditPlan,
  onResend,
  pendingAskUser,
  onRespondToAskUser,
  onCancelAgentRun,
  isRunning,
  verboseMode,
  overlay,
  onDismissOverlay,
  modelPickerOpen,
  models: modelsList,
  defaultModels: defaultModelsList,
  onSwitchModel,
  tokensPerSec,
  mobile,
  scrollToBottom: scrollToBottomProp,
  showScrollButton: showScrollButtonProp,
  showSubagentPane = true,
}) => {
  const [openSubagentId, setOpenSubagentId] = useState<string | null>(null);
  const [subagentMessageFilter, setSubagentMessageFilter] = useState('');

  // Every surface that renders chat wants the user's core-memory name for
  // bubble labels — loaded here so the embed and consumer shells get it too.
  useEffect(() => {
    useUserStore.getState().loadCoreName();
  }, []);

  const [expandedMessages, setExpandedMessages] = useState<Set<string>>(new Set());
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const chatScrollRef = useRef<HTMLDivElement | null>(null);
  const userMsgRefs = useRef<Map<number, HTMLDivElement>>(new Map());

  // Focus chat input when session changes (e.g. new session created)
  useEffect(() => {
    if (!sessionId) return;
    // Small delay to let the DOM settle after session switch
    const t = setTimeout(() => inputRef.current?.focus(), 50);
    return () => clearTimeout(t);
  }, [sessionId]);

  useEffect(() => {
    if (pendingAskUser) chatEndRef?.current?.scrollIntoView({ behavior: 'auto', block: 'nearest' });
  }, [pendingAskUser, chatEndRef]);

  // Auto-scroll is handled by useAutoScroll in ChatWidget; it hands down
  // scrollToBottom and showScrollButton.
  const showScrollButton = showScrollButtonProp ?? false;
  const scrollToBottom = scrollToBottomProp ?? (() => {
    chatEndRef?.current?.scrollIntoView({ behavior: 'smooth', block: 'end' });
  });

  const agentStatusText = useServerStore((s) => s.agentStatusText);
  const reconnecting = useUserStore((s) => s.connectionStatus === 'reconnecting');
  const { isAgentActive, spinnerVerb, thinkingElapsed, lastRunSummary } = useRunSpinner(sessionId);

  const mainAgentIds = useMemo(() => mainAgents.map((agent) => normalizeAgentKey(agent.name)), [mainAgents]);

  const {
    filteredMainMessages, historicalMessages, streamingMessage, visibleQueued, selectedSubagent, filteredSubagentMessages,
  } = useChatFilters({ chatMessages, queuedMessages, selectedAgent, mainAgentIds, subagents, openSubagentId, subagentMessageFilter });
  const floatingUserMsg = useFloatingUserMessage(chatScrollRef, userMsgRefs, filteredMainMessages, sessionId);
  const { from: windowFrom, loadEarlier } = useMessageWindow(historicalMessages.length, sessionId, chatScrollRef);
  const { askUserBelongsToSubagent, paneVisible, closePane } = useSubagentPane(filteredMainMessages, pendingAskUser, selectedAgent);

  // What "stop" acts on. The selected agent's run when there is one, else
  // whatever is running in this session: the two disagree whenever the run
  // belongs to an agent the surface never selected, and a stop button that
  // vanishes exactly while something is running is worse than none.
  const selectedMainRunningRunId =
    runningMainRunIds?.[normalizeAgentKey(selectedAgent)] ||
    (sessionId ? runningRunIdBySession?.[sessionId] : undefined);

  const streamingPlanProps = useMemo(
    () => ({ pendingPlanAgentId, agentContext, onApprovePlan, onRejectPlan, onEditPlan, inputRef }),
    [pendingPlanAgentId, agentContext, onApprovePlan, onRejectPlan, onEditPlan, inputRef],
  );

  // A drawer for a subagent that no longer exists closes.
  useEffect(() => {
    if (openSubagentId && !subagents.some((sub) => sub.id === openSubagentId)) setOpenSubagentId(null);
  }, [openSubagentId, subagents]);

  return (
    <section className="h-full flex flex-col bg-white dark:bg-[#0f0f0f] rounded-xl border border-slate-200 dark:border-white/5 overflow-hidden min-h-0 relative">
      <div className="px-1.5 py-1 border-b border-slate-200 dark:border-white/5 bg-slate-50/70 dark:bg-white/[0.02] space-y-1">
        {sessionId && (
          <details className="rounded-md border border-slate-200 dark:border-white/10 bg-white/80 dark:bg-black/20 px-2 py-1 text-[11px] text-slate-600 dark:text-slate-300">
            <summary className="cursor-pointer flex flex-wrap items-center gap-2">
              {sessionId && (() => {
                const sessionMeta = useSessionStore.getState().allSessions.find(s => s.id === sessionId);
                return (
                  <>
                    <span className="font-semibold uppercase tracking-wider text-slate-500">Session</span>
                    <span className="font-mono truncate max-w-[160px]">{sessionId}</span>
                    {/* Paths are hidden from room consumers, not from this
                        machine's own devices — a paired peer is admin. */}
                    {useUserStore.getState().userType !== 'consumer' && (sessionMeta?.project_name || sessionMeta?.cwd) && (() => {
                      const fullPath = sessionMeta?.cwd || sessionMeta?.project || '';
                      const displayName = sessionMeta?.project_name || fullPath.split('/').filter(Boolean).pop() || fullPath;
                      return (
                        <span className="px-1.5 py-0.5 rounded bg-slate-100 dark:bg-white/5 text-slate-500 text-[10px] shrink-0" title={fullPath}>
                          📁 {displayName}
                        </span>
                      );
                    })()}
                    {sessionMeta?.creator && sessionMeta.creator !== 'user' && (
                      <span className={cn('px-1.5 py-0.5 rounded text-[10px] font-medium',
                        sessionMeta.creator === 'mission' ? 'bg-amber-100 dark:bg-amber-500/10 text-amber-600 dark:text-amber-400'
                          : 'bg-purple-100 dark:bg-purple-500/10 text-purple-600 dark:text-purple-400'
                      )}>
                        {sessionMeta.creator === 'mission' ? '🤖' : '✨'} {sessionMeta.creator}
                      </span>
                    )}
                    {sessionMeta?.skill && sessionMeta.creator !== 'mission' && (
                      <span className="px-1.5 py-0.5 rounded bg-purple-50 dark:bg-purple-500/5 text-purple-500 text-[10px]">
                        {sessionMeta.skill}
                      </span>
                    )}
                  </>
                );
              })()}
              <SessionModeSelector />
              <SessionModelSelector />
              <SessionStats />
            </summary>
            <div className="mt-1.5 flex flex-wrap items-center justify-between gap-2">
              <div className="flex flex-wrap items-center gap-2">
                {selectedMainRunningRunId && onCancelRun && (
                  <button
                    onClick={() => onCancelRun(selectedMainRunningRunId)}
                    disabled={!!cancellingRunIds?.[selectedMainRunningRunId]}
                    className={cn(
                      'px-2 py-1 rounded border text-[11px] font-semibold transition-colors',
                      cancellingRunIds?.[selectedMainRunningRunId]
                        ? 'bg-slate-100 text-slate-400 border-slate-200 cursor-not-allowed'
                        : 'bg-red-50 text-red-600 border-red-200 hover:bg-red-100'
                    )}
                    title={selectedMainRunningRunId}
                  >
                    {cancellingRunIds?.[selectedMainRunningRunId] ? 'Cancelling...' : 'Cancel Run'}
                  </button>
                )}
              </div>
              <ChatDebugActions projectRoot={projectRoot} sessionId={sessionId} />
            </div>
          </details>
        )}

        {subagents.length > 0 && (
          <details className="rounded-md border border-slate-200 dark:border-white/10 bg-white/80 dark:bg-black/20 px-2 py-1 text-[11px]">
            <summary className="cursor-pointer font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400 flex items-center gap-1">
              <Sparkles size={11} />
              Subagents ({subagents.length})
            </summary>
            <div className="mt-1 flex flex-wrap items-center gap-1.5">
              {subagents.map((sub) => (
                <button
                  key={sub.id}
                  onClick={() => setOpenSubagentId(sub.id)}
                  className={cn(
                    'px-2 py-1 rounded-md text-[11px] border transition-colors flex items-center gap-1',
                    openSubagentId === sub.id
                      ? 'bg-blue-600 text-white border-blue-600'
                      : 'bg-slate-100 dark:bg-white/5 border-slate-200 dark:border-white/10 hover:bg-slate-200 dark:hover:bg-white/10'
                  )}
                >
                  <span className="font-semibold">{sub.id}</span>
                  <span className={cn('px-1.5 py-0.5 rounded-full uppercase tracking-wide', statusBadgeClass(sub.status))}>
                    {sub.status}
                  </span>
                </button>
              ))}
            </div>
          </details>
        )}
      </div>

      <div className="relative flex-1 flex flex-col md:flex-row gap-0 min-h-0">
      <div
        ref={chatScrollRef}
        className={cn(
          'relative overflow-y-scroll px-2 py-1.5 flex flex-col gap-2 custom-scrollbar min-h-0',
          // 60/40 split on wide when the pane is shown; full width when it's not.
          showSubagentPane && paneVisible
            ? 'flex-1 md:flex-[3] md:min-w-0'
            : 'flex-1',
        )}
      >
        {floatingUserMsg && (
          <div
            className="sticky top-0 z-20 mx-1 mb-1 px-3 py-2 rounded-md bg-slate-100/95 dark:bg-white/10 backdrop-blur text-[13px] text-slate-700 dark:text-slate-200 border border-slate-200/60 dark:border-white/10 cursor-pointer"
            onClick={() => {
              const el = userMsgRefs.current.get(floatingUserMsg.index);
              el?.scrollIntoView({ behavior: 'smooth', block: 'center' });
            }}
            title={floatingUserMsg.text}
          >
            <p className="line-clamp-4 whitespace-pre-wrap break-words">
              <span className="font-medium text-slate-500 dark:text-slate-400 mr-1.5">You:</span>
              {floatingUserMsg.text}
            </p>
          </div>
        )}
        <ChatMessageList
          messages={historicalMessages}
          from={windowFrom}
          onLoadEarlier={loadEarlier}
          expandedMessages={expandedMessages}
          setExpandedMessages={setExpandedMessages}
          verboseMode={verboseMode}
          userMsgRefs={userMsgRefs}
          selectedAgent={selectedAgent}
          pendingPlanAgentId={pendingPlanAgentId}
          agentContext={agentContext}
          onApprovePlan={onApprovePlan}
          onRejectPlan={onRejectPlan}
          onEditPlan={onEditPlan}
          onResend={onResend}
          inputRef={inputRef}
        />
        {streamingMessage && (
          <ChatMessageRow
            msg={streamingMessage}
            msgKey={`${streamingMessage.timestamp}-${filteredMainMessages.length - 1}-${streamingMessage.from || streamingMessage.role}-${streamingMessage.text.slice(0, 24)}`}
            isUser={false}
            isExpanded={verboseMode || false}
            planProps={streamingPlanProps}
          />
        )}
        {pendingAskUser && onRespondToAskUser && !askUserBelongsToSubagent && (
          <div className="px-3 py-2">
            {pendingAskUser.questions[0]?.header === 'Permission'
              ? <ToolPermissionCard pending={pendingAskUser} onRespond={onRespondToAskUser} />
              : <AskUserCard pending={pendingAskUser} onRespond={onRespondToAskUser} />}
          </div>
        )}
        <div ref={chatEndRef} />
        {showScrollButton && (
          <button
            onClick={scrollToBottom}
            className="sticky bottom-2 left-1/2 -translate-x-1/2 z-30 w-8 h-8 rounded-full bg-white dark:bg-[#1a1a1a] border border-slate-300 dark:border-white/15 shadow-lg flex items-center justify-center hover:bg-slate-50 dark:hover:bg-white/10 transition-all opacity-80 hover:opacity-100"
            title="Scroll to bottom"
          >
            <ArrowDown size={14} className="text-slate-600 dark:text-slate-300" />
          </button>
        )}
      </div>

      {showSubagentPane && paneVisible && (
        <div className="md:flex-[2] md:min-w-0 md:max-w-[40%] border-t md:border-t-0 border-slate-200 dark:border-white/10 min-h-[180px] md:min-h-0">
          <SubagentPane
            messages={filteredMainMessages}
            visible={paneVisible}
            pendingAskUser={askUserBelongsToSubagent ? pendingAskUser : null}
            onRespondToAskUser={onRespondToAskUser}
            onCancelAgentRun={onCancelAgentRun}
            onClose={closePane}
          />
        </div>
      )}
      </div>

      <RunStatusLine
        active={isAgentActive}
        reconnecting={reconnecting}
        label={agentStatusText?.[sessionId || ''] || spinnerVerb || 'Thinking'}
        elapsed={thinkingElapsed}
        tokensPerSec={tokensPerSec}
        context={agentContext?.[sessionId || '']}
        summary={lastRunSummary}
      />
      <ChatInput
        projectRoot={projectRoot}
        selectedAgent={selectedAgent}
        setSelectedAgent={setSelectedAgent}
        skills={skills}
        agents={agents}
        mainAgentIds={mainAgentIds}
        isRunning={isRunning}
        onSendMessage={onSendMessage}
        onCancelAgentRun={onCancelAgentRun}
        selectedMainRunningRunId={selectedMainRunningRunId}
        activePlan={activePlan}
        visibleQueued={visibleQueued}
        sessionId={sessionId}
        openQuestion={pendingAskUser && !askUserBelongsToSubagent && pendingAskUser.questions[0]?.header !== 'Permission' ? pendingAskUser : null}
        onAnswerQuestion={onRespondToAskUser}
        overlay={overlay}
        onDismissOverlay={onDismissOverlay}
        inputRef={inputRef}
        modelPickerOpen={modelPickerOpen}
        models={modelsList}
        defaultModels={defaultModelsList}
        onSwitchModel={onSwitchModel}
        mobile={mobile}
      />

      {selectedSubagent && (
        <SubagentDrawer
          selectedSubagent={selectedSubagent}
          filteredSubagentMessages={filteredSubagentMessages}
          subagentMessageFilter={subagentMessageFilter}
          setSubagentMessageFilter={setSubagentMessageFilter}
          cancellingRunIds={cancellingRunIds}
          onCancelRun={onCancelRun}
          onClose={() => setOpenSubagentId(null)}
        />
      )}
    </section>
  );
};
