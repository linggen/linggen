import React, { useMemo, useEffect, useRef, useCallback } from 'react';
import { Bot, FilePenLine, Plus, RefreshCw, Settings, Sparkles, Zap } from 'lucide-react';
import { cn } from './lib/cn';
import { AgentsCard } from './components/AgentsCard';
import { SessionNav } from './components/SessionNav';
import { ModelsCard } from './components/ModelsCard';
import { CollapsibleCard } from './components/CollapsibleCard';
import { SkillsCard } from './components/SkillsCard';
import { FilePreview } from './components/FilePreview';
import { ChatPanel } from './components/chat';
import { HeaderBar } from './components/HeaderBar';
import { SettingsPage } from './components/SettingsPage';
import { MissionSessionNav } from './components/MissionSessionNav';
import { MissionEditor } from './components/MissionPage';
import { AgentSpecEditorModal } from './components/AgentSpecEditorModal';
import { ToastContainer } from './components/ToastContainer';
import { AppPanel } from './components/AppPanel';
import type { AgentRunInfo, AgentRunSummary } from './types';
import {
  buildAgentWorkInfo,
  buildSubagentInfos,
} from './lib/messageUtils';
import { useProjectStore } from './stores/projectStore';
import { useAgentStore } from './stores/agentStore';
import { useChatStore } from './stores/chatStore';
import { useUiStore } from './stores/uiStore';
import { useSseConnection } from './hooks/useSseConnection';
import { useSseDispatch } from './hooks/useSseDispatch';

// ---------------------------------------------------------------------------
// Auto-scroll hook (needs DOM refs — stays in React)
// ---------------------------------------------------------------------------

function useAutoScroll(messages: { length: number }, lastMsg: { isGenerating?: boolean; content?: any[] } | undefined) {
  const chatEndRef = useRef<HTMLDivElement>(null);
  const lastChatCountRef = useRef(0);
  const lastContentLenRef = useRef(0);
  const isNearBottomRef = useRef(true);
  const chatScrollContainerRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const endEl = chatEndRef.current;
    if (!endEl) return;
    const container = endEl.parentElement;
    if (!container) return;
    chatScrollContainerRef.current = container;
    const onScroll = () => {
      const { scrollTop, scrollHeight, clientHeight } = container;
      const distanceFromBottom = scrollHeight - scrollTop - clientHeight;
      const threshold = scrollHeight * 0.1;
      isNearBottomRef.current = distanceFromBottom <= Math.max(threshold, 80);
    };
    container.addEventListener('scroll', onScroll, { passive: true });
    return () => container.removeEventListener('scroll', onScroll);
  }, []);

  const lastContentLen = lastMsg?.isGenerating ? (lastMsg.content?.length || 0) : 0;
  useEffect(() => {
    const newMessages = messages.length > lastChatCountRef.current;
    const newContentBlocks = lastContentLen > lastContentLenRef.current;
    if ((newMessages || newContentBlocks) && isNearBottomRef.current) {
      chatEndRef.current?.scrollIntoView({ behavior: 'auto', block: 'nearest', inline: 'nearest' });
    }
    lastChatCountRef.current = messages.length;
    lastContentLenRef.current = lastContentLen;
  }, [messages.length, lastContentLen]);

  const scrollToBottom = useCallback(() => {
    isNearBottomRef.current = true;
    chatEndRef.current?.scrollIntoView({ behavior: 'auto', block: 'nearest', inline: 'nearest' });
  }, []);

  return { chatEndRef, scrollToBottom };
}

// ---------------------------------------------------------------------------
// Compact mode (VS Code sidebar)
// ---------------------------------------------------------------------------

const compactParams = new URLSearchParams(window.location.search);
const isCompact = compactParams.get('mode') === 'compact';
const compactProject = compactParams.get('project') || '';

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

const App: React.FC = () => {
  // --- Stores ---
  const projectStore = useProjectStore();
  const agentStore = useAgentStore();
  const chatStore = useChatStore();
  const uiStore = useUiStore();

  // Shortcuts
  const { projects, selectedProjectRoot, sessions, activeSessionId, isMissionSession, agentTreesByProject } = projectStore;
  const { agents, models, skills, agentRuns, selectedAgent, agentStatus, agentStatusText, agentContext, defaultModels, ollamaStatus, sessionTokens, tokensPerSec, cancellingRunIds, reloadingSkills } = agentStore;
  const { displayMessages, messages: chatMessages } = chatStore;
  const { currentPage, sidebarTab, editingMission, missionRefreshKey, overlay, modelPickerOpen, showAgentSpecEditor, openApp, selectedFileContent, selectedFilePath, queuedMessages, pendingPlan, pendingPlanAgentId, pendingAskUser, activePlan, verboseMode, copyChatStatus } = uiStore;

  const isRunning = agentStore.isRunning();
  const mainAgents = agents;

  // --- Auto-scroll ---
  const lastMsg = chatMessages[chatMessages.length - 1];
  const { chatEndRef, scrollToBottom } = useAutoScroll(chatMessages, lastMsg);

  // Session-change tracking (for clear-on-switch)
  const prevSessionIdRef = useRef<string | null>(null);

  // --- Derived memos ---
  const activeModelId = useMemo(() => {
    for (const name of Object.keys(agentStatusText)) {
      const status = agentStatus[name];
      if (status && status !== 'idle') {
        const text = agentStatusText[name] || '';
        const match = text.match(/\(([^)]+)\)/);
        if (match) return match[1];
      }
    }
    return undefined;
  }, [agentStatus, agentStatusText]);

  const mainAgentIds = useMemo(() => agents.map((a) => a.name.toLowerCase()), [agents]);

  const agentTree = useMemo(
    () => agentTreesByProject[selectedProjectRoot] || {},
    [agentTreesByProject, selectedProjectRoot],
  );
  const agentWork = useMemo(() => buildAgentWorkInfo(agentTree), [agentTree]);
  const subagents = useMemo(
    () => buildSubagentInfos(agentTree, mainAgentIds, agentStatus),
    [agentTree, mainAgentIds, agentStatus],
  );

  const sortedAgentRuns = useMemo(() => {
    const statusScore = (status: string) => (status === 'running' ? 1 : 0);
    return [...agentRuns].sort(
      (a, b) => statusScore(b.status) - statusScore(a.status) || Number(b.started_at || 0) - Number(a.started_at || 0),
    );
  }, [agentRuns]);

  const mainRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);

  const subagentRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (!run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);

  const runningMainRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.parent_run_id || run.status !== 'running') continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);

  const runningSubagentRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (!run.parent_run_id || run.status !== 'running') continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);

  const mainRunHistory = useMemo(() => {
    const out: Record<string, AgentRunInfo[]> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = [];
      out[agentId].push(run);
    }
    return out;
  }, [sortedAgentRuns]);

  const subagentRunHistory = useMemo(() => {
    const out: Record<string, AgentRunInfo[]> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (!run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = [];
      out[agentId].push(run);
    }
    return out;
  }, [sortedAgentRuns]);

  const agentRunSummary = useMemo(() => {
    const out: Record<string, AgentRunSummary> = {};
    for (const agent of agents) {
      const agentId = agent.name.toLowerCase();
      const latest = mainRunHistory[agentId]?.[0];
      if (!latest) continue;
      const children = sortedAgentRuns.filter((run) => run.parent_run_id === latest.run_id);
      const timelineEvents = 1 + (latest.ended_at ? 1 : 0) + children.reduce((count, child) => count + 1 + (child.ended_at ? 1 : 0), 0);
      const lastEventAt = Math.max(
        Number(latest.ended_at || 0), Number(latest.started_at || 0),
        ...children.flatMap((child) => [Number(child.started_at || 0), Number(child.ended_at || 0)]),
      );
      out[agentId] = {
        run_id: latest.run_id,
        status: latest.status,
        started_at: latest.started_at,
        ended_at: latest.ended_at,
        child_count: children.length,
        timeline_events: timelineEvents,
        last_event_at: lastEventAt,
      };
    }
    return out;
  }, [agents, mainRunHistory, sortedAgentRuns]);

  // --- SSE ---
  const handleSseEvent = useSseDispatch();
  useSseConnection({
    onEvent: handleSseEvent,
    onParseError: () => { chatStore.fetchWorkspaceState(); agentStore.fetchAgentRuns(); },
    sessionId: activeSessionId,
  });

  // --- Initial data load ---
  useEffect(() => {
    useProjectStore.getState().fetchProjects();
    useAgentStore.getState().fetchSkills();
    useAgentStore.getState().fetchAgents();
    useAgentStore.getState().fetchModels();
    useAgentStore.getState().fetchDefaultModels();

    const interval = setInterval(() => { useAgentStore.getState().fetchOllamaStatus(); useAgentStore.getState().fetchSessionTokens(); }, 5000);
    useAgentStore.getState().fetchOllamaStatus();
    useAgentStore.getState().fetchSessionTokens();
    return () => clearInterval(interval);
  }, []);

  // --- React to selected project changes ---
  useEffect(() => {
    if (selectedProjectRoot) {
      useProjectStore.getState().fetchFiles();
      useChatStore.getState().fetchWorkspaceState();
      useProjectStore.getState().fetchAgentTree(selectedProjectRoot);
      useAgentStore.getState().fetchAgentRuns();
      useProjectStore.getState().fetchSessions();
      useAgentStore.getState().fetchAgents(selectedProjectRoot);
      useAgentStore.getState().resetStatus();
      useUiStore.getState().setQueuedMessages([]);
      useUiStore.getState().setActivePlan(null);
    }
  }, [selectedProjectRoot]);

  // --- React to projects list changes ---
  useEffect(() => {
    if (projects.length > 0) {
      useProjectStore.getState().fetchAllAgentTrees();
      useProjectStore.getState().fetchAllSessionCounts();
    }
  }, [projects]);

  // Fetch any pending AskUser/permission prompts from the server.
  const fetchPendingAskUser = useCallback(async () => {
    try {
      const resp = await fetch('/api/pending-ask-user');
      if (!resp.ok) return;
      const items = await resp.json();
      if (Array.isArray(items) && items.length > 0) {
        // Pick the first pending question (there's typically at most one active).
        const item = items[0];
        useUiStore.getState().setPendingAskUser({
          questionId: item.question_id,
          agentId: item.agent_id,
          questions: item.questions,
        });
      } else {
        useUiStore.getState().setPendingAskUser(null);
      }
    } catch { /* ignore */ }
  }, []);

  // --- React to session changes ---
  useEffect(() => {
    if (selectedProjectRoot || isMissionSession) {
      const prev = prevSessionIdRef.current;
      prevSessionIdRef.current = activeSessionId;
      const isSessionAdoption = prev === null && activeSessionId !== null;
      if (!isSessionAdoption) {
        useChatStore.getState().clear(false);
        const ui = useUiStore.getState();
        ui.setQueuedMessages([]);
        ui.setActivePlan(null);
        ui.setPendingPlan(null);
        ui.setPendingPlanAgentId(null);
        ui.setSessionModel(null);
      }
      useChatStore.getState().fetchWorkspaceState();
      useAgentStore.getState().fetchAgentRuns();
      // Restore any pending AskUser/permission widget for this session.
      fetchPendingAskUser();
    }
  }, [activeSessionId, selectedProjectRoot, isMissionSession]);

  // --- Poll workspace state for mission sessions (backup; SSE also triggers reloads) ---
  useEffect(() => {
    if (!isMissionSession || !activeSessionId) return;
    const interval = setInterval(() => {
      useChatStore.getState().fetchWorkspaceState();
    }, 5000);
    return () => clearInterval(interval);
  }, [isMissionSession, activeSessionId]);

  // --- Auto-select agent ---
  useEffect(() => {
    if (mainAgentIds.length === 0) return;
    if (!mainAgentIds.includes(selectedAgent.toLowerCase())) {
      const preferred = mainAgentIds.includes('ling') ? 'ling' : mainAgentIds[0];
      useAgentStore.getState().setSelectedAgent(preferred);
    }
  }, [mainAgentIds, selectedAgent]);

  // --- Compact mode init ---
  const compactSessionInitRef = useRef(false);
  useEffect(() => {
    if (!isCompact || !compactProject || compactSessionInitRef.current) return;
    compactSessionInitRef.current = true;
    useProjectStore.getState().setSelectedProjectRoot(compactProject);
    (async () => {
      try {
        const resp = await fetch(`/api/sessions?project_root=${encodeURIComponent(compactProject)}`);
        const data = await resp.json();
        const sessionList = data.sessions ?? data ?? [];
        const existing = sessionList.find((s: any) => s.title?.startsWith('VS Code'));
        if (existing) {
          useProjectStore.getState().setActiveSessionId(existing.id);
        } else {
          const createResp = await fetch('/api/sessions', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ project_root: compactProject, title: 'VS Code' }),
          });
          const created = await createResp.json();
          useProjectStore.getState().setActiveSessionId(created.id);
        }
      } catch (e) {
        console.error('Compact session init error:', e);
      }
    })();
  }, []);

  // --- Clipboard bridge for VS Code compact mode ---
  useEffect(() => {
    if (!isCompact) return;
    document.documentElement.setAttribute('data-compact', '');
    const handleMessage = (e: MessageEvent) => {
      if (e.data?.type !== 'linggen-clipboard') return;
      const sel = window.getSelection();
      const text = sel?.toString();
      switch (e.data.action) {
        case 'copy': if (text) navigator.clipboard.writeText(text).catch(() => {}); break;
        case 'cut': if (text) { navigator.clipboard.writeText(text).catch(() => {}); document.execCommand('delete'); } break;
        case 'selectAll': document.execCommand('selectAll'); break;
      }
    };
    const handleCopy = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 'c' && !e.shiftKey && !e.altKey) {
        const sel = window.getSelection();
        const text = sel?.toString();
        if (text) { e.preventDefault(); navigator.clipboard.writeText(text).catch(() => {}); }
      }
    };
    window.addEventListener('message', handleMessage);
    document.addEventListener('keydown', handleCopy);
    return () => { window.removeEventListener('message', handleMessage); document.removeEventListener('keydown', handleCopy); };
  }, []);

  // --- Chat actions ---
  const clearChat = useCallback(async () => {
    const { selectedProjectRoot: root, activeSessionId: sid } = useProjectStore.getState();
    if (!root) return;
    useChatStore.getState().clear();
    const ui = useUiStore.getState();
    ui.setQueuedMessages([]);
    ui.setActivePlan(null);
    ui.setPendingPlan(null);
    ui.setPendingPlanAgentId(null);
    try {
      const resp = await fetch('/api/chat/clear', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: root, session_id: sid }),
      });
      if (!resp.ok) console.error('Clear chat API error:', resp.status);
      useChatStore.getState().clear();
    } catch (e) { console.error('Error clearing chat:', e); }
  }, []);

  const sendChatMessage = useCallback(async (userMessage: string, targetAgent?: string, images?: string[]) => {
    if (!userMessage.trim() && !(images && images.length > 0)) return;
    const { selectedProjectRoot: root, activeSessionId: sid } = useProjectStore.getState();
    const agent = useAgentStore.getState().selectedAgent;
    if (!root) return;
    const agentToUse = targetAgent || agent;
    if (!agentToUse) return;
    const now = new Date();
    const trimmed = userMessage.trim();
    const ui = useUiStore.getState();
    const chat = useChatStore.getState();

    if (trimmed !== '/help' && trimmed !== '/status' && trimmed !== '/clear' && trimmed !== '/compact' && !trimmed.startsWith('/compact ') && !trimmed.startsWith('/model') && !trimmed.startsWith('!')) {
      chat.addMessage({
        role: 'user', from: 'user', to: agentToUse, text: userMessage,
        timestamp: now.toLocaleTimeString(), timestampMs: now.getTime(), isGenerating: false,
        ...(images && images.length > 0 ? { images } : {}),
      });
      scrollToBottom();
    }

    // /model
    if (trimmed === '/model' || trimmed.startsWith('/model ')) {
      const modelArg = trimmed.slice('/model'.length).trim();
      if (!modelArg) { ui.setModelPickerOpen(true); ui.setOverlay(null); }
      else {
        const currentModels = useAgentStore.getState().models;
        const valid = currentModels.length === 0 || currentModels.some((m) => m.id === modelArg);
        if (!valid) { ui.setOverlay(`Unknown model: \`${modelArg}\`. Use \`/model\` to see available models.`); }
        else {
          try {
            const resp = await fetch('/api/config');
            if (resp.ok) {
              const config = await resp.json();
              const newDefaults = [modelArg];
              const updated = { ...config, routing: { ...config.routing, default_models: newDefaults } };
              const saveResp = await fetch('/api/config', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(updated) });
              if (saveResp.ok) { useAgentStore.setState({ defaultModels: newDefaults }); ui.setOverlay(`Switched default model to: \`${modelArg}\``); }
            }
          } catch (e) { ui.setOverlay(`Error switching model: ${e}`); }
        }
      }
      return;
    }

    if (trimmed === '/help') {
      ui.setOverlay([
        '**Commands:**', '- `/help` — Show available commands', '- `/clear` — Clear chat context',
        '- `/compact [focus]` — Compact context (summarize old messages)',
        '- `/status` — Show project status', '- `/model` — List models; `/model <id>` — Switch default model',
        '- `/plan <task>` — Ask agent to create a plan (read-only)', '- `/image <path>` — Attach an image file',
        '- `!command` — Run a shell command directly',
        '- `@path` — Mention a file', '- `@@agent message` — Send to specific agent', '', '**Skills:** Type `/` to see available skills.',
      ].join('\n'));
      return;
    }

    if (trimmed === '/status') {
      try {
        const resp = await fetch(`/api/status?project_root=${encodeURIComponent(root)}`);
        if (resp.ok) {
          const data = await resp.json();
          const modelLines = (data.models || []).map((m: any) => `- \`${m.id}${m.id === data.default_model ? ' ✓' : ''}\`  (${m.provider}: ${m.model})`);
          const usageLines = (data.model_usage || []).map((entry: [string, number]) => `- \`${entry[0]}\` — ${entry[1]} runs`);
          const fmt = (n: number) => n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1_000 ? `${(n / 1_000).toFixed(1)}K` : `${n}`;
          const promptTok = data.session_prompt_tokens || 0;
          const completionTok = data.session_completion_tokens || 0;
          const lines = [
            `**Version:** v${data.version || '?'}`, `**Session:** \`${sid || '(none)'}\``,
            `**Workspace:** \`${root}\``, `**Agent:** ${agent}`,
            `**Model:** \`${data.default_model || '(none)'}\``,
          ];
          if (promptTok > 0 || completionTok > 0) lines.push(`**Tokens:** ↑ ${fmt(promptTok)}  ↓ ${fmt(completionTok)}  (total: ${fmt(promptTok + completionTok)})`);
          lines.push('', '**Models:**', ...modelLines, '', '| Metric | Value |', '|--------|-------|',
            `| Sessions | ${data.sessions} |`, `| Total runs | ${data.total_runs} |`,
            `| Completed | ${data.completed_runs} |`, `| Failed | ${data.failed_runs} |`,
            `| Cancelled | ${data.cancelled_runs} |`, `| Active days | ${data.active_days} |`);
          if (usageLines.length > 0) lines.push('', '**Model usage:**', ...usageLines);
          ui.setOverlay(lines.join('\n'));
        } else { ui.setOverlay(`Status request failed: ${resp.status} ${resp.statusText}`); }
      } catch (e) { ui.setOverlay(`Error fetching status: ${e}`); }
      return;
    }

    if (trimmed === '/clear') { await clearChat(); return; }

    // ! prefix — direct bash execution (CC-style)
    if (trimmed.startsWith('!') && trimmed.length > 1) {
      const cmd = trimmed.slice(1);
      const ts = new Date();
      chat.addMessage({
        role: 'user', from: 'user', to: 'system', text: `\`$ ${cmd}\``,
        timestamp: ts.toLocaleTimeString(), timestampMs: ts.getTime(), isGenerating: false,
      });
      scrollToBottom();
      try {
        const resp = await fetch('/api/bash', {
          method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ project_root: root, command: cmd }),
        });
        const data = await resp.json();
        const output = [data.stdout, data.stderr].filter(Boolean).join('\n').trim();
        const exitInfo = data.exit_code !== 0 ? `\n\n(exit code ${data.exit_code})` : '';
        const resultTs = new Date();
        chat.addMessage({
          role: 'assistant', from: 'system', to: 'user',
          text: output ? `\`\`\`\n${output}\n\`\`\`${exitInfo}` : `(no output)${exitInfo}`,
          timestamp: resultTs.toLocaleTimeString(), timestampMs: resultTs.getTime(), isGenerating: false,
        });
        scrollToBottom();
      } catch (e) {
        console.error('Bash error:', e);
      }
      return;
    }

    if (trimmed === '/compact' || trimmed.startsWith('/compact ')) {
      const focus = trimmed.slice('/compact'.length).trim() || undefined;
      const { setAgentStatus, setAgentStatusText } = useAgentStore.getState();
      setAgentStatus((s) => ({ ...s, [agentToUse]: 'thinking' as const }));
      setAgentStatusText((s) => ({ ...s, [agentToUse]: 'Compacting conversation' }));
      try {
        const resp = await fetch('/api/chat/compact', {
          method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ project_root: root, session_id: sid, agent_id: agentToUse, focus }),
        });
        const data = await resp.json();
        const clearStatus = () => {
          setAgentStatus((s) => ({ ...s, [agentToUse]: 'idle' as const }));
          setAgentStatusText((s) => { const n = { ...s }; delete n[agentToUse]; return n; });
        };
        clearStatus();
        if (data.compacted) {
          // Clear UI and reload compacted state from rewritten session file.
          useChatStore.getState().clear(false);
          await useChatStore.getState().fetchWorkspaceState();
          // Add CC-style "Conversation compacted" banner with referenced files.
          const refs = (data.referenced_files || []) as string[];
          const refsText = refs.length > 0
            ? '\n\n' + refs.map((f: string) => `Referenced file ${f}`).join('\n')
            : '';
          const ts = new Date();
          useChatStore.getState().addMessage({
            role: 'assistant', from: 'system', to: 'user',
            text: `Conversation compacted.${refsText}`,
            timestamp: ts.toLocaleTimeString(), timestampMs: ts.getTime(), isGenerating: false,
          });
        } else {
          const ts = new Date();
          useChatStore.getState().addMessage({
            role: 'assistant', from: agentToUse, to: 'user', text: 'Nothing to compact.',
            timestamp: ts.toLocaleTimeString(), timestampMs: ts.getTime(), isGenerating: false,
          });
        }
        scrollToBottom();
      } catch (e) {
        setAgentStatus((s) => ({ ...s, [agentToUse]: 'idle' as const }));
        setAgentStatusText((s) => { const n = { ...s }; delete n[agentToUse]; return n; });
        console.error('Compact error:', e);
      }
      return;
    }

    if (trimmed.startsWith('/user_story ')) {
      await fetch('/api/task', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: root, agent_id: agentToUse, task: trimmed.substring(12).trim() }),
      });
      return;
    }

    try {
      const { isMissionSession, activeMissionId } = useProjectStore.getState();
      const resp = await fetch('/api/chat', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: root, agent_id: agentToUse, message: userMessage,
          session_id: sid,
          ...(isMissionSession && activeMissionId ? { mission_id: activeMissionId } : {}),
          ...(useUiStore.getState().sessionModel ? { model_id: useUiStore.getState().sessionModel } : {}),
          ...(images && images.length > 0 ? { images } : {}),
        }),
      });
      const data = await resp.json();
      if (data?.session_id && !sid) {
        useProjectStore.getState().setActiveSessionId(data.session_id);
        useProjectStore.getState().fetchSessions();
      }
      if (data?.status === 'queued') {
        useChatStore.getState().removeLastUserMessage(userMessage, agentToUse);
        return;
      }
      useAgentStore.getState().setAgentStatus((prev) => ({ ...prev, [agentToUse]: 'model_loading' }));
      useAgentStore.getState().setAgentStatusText((prev) => ({ ...prev, [agentToUse]: 'Model Loading' }));
      useChatStore.getState().upsertGenerating(agentToUse, 'Model loading...', 'Model loading...');
    } catch (e) {
      console.error('Error in chat:', e);
    }
  }, [scrollToBottom, clearChat]);

  const respondToAskUser = useCallback(async (questionId: string, answers: any[]) => {
    try {
      await fetch('/api/ask-user-response', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ question_id: questionId, answers }),
      });
      useUiStore.getState().setPendingAskUser(null);
    } catch (e) { console.error('Error responding to AskUser:', e); }
  }, []);

  const approvePlan = useCallback(async (clearContext = false) => {
    const { pendingPlanAgentId: planAgent } = useUiStore.getState();
    const { selectedProjectRoot: root, activeSessionId: sid } = useProjectStore.getState();
    if (!planAgent || !root) return;
    try {
      await fetch('/api/plan/approve', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: root, agent_id: planAgent, session_id: sid, clear_context: clearContext }),
      });
      const ui = useUiStore.getState();
      ui.setPendingPlan(null);
      ui.setPendingPlanAgentId(null);
    } catch (e) { console.error('Error approving plan:', e); }
  }, []);

  const rejectPlan = useCallback(async () => {
    const { pendingPlanAgentId: planAgent } = useUiStore.getState();
    const { selectedProjectRoot: root, activeSessionId: sid } = useProjectStore.getState();
    if (!planAgent || !root) return;
    try {
      await fetch('/api/plan/reject', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: root, agent_id: planAgent, session_id: sid }),
      });
      const ui = useUiStore.getState();
      ui.setPendingPlan(null);
      ui.setPendingPlanAgentId(null);
    } catch (e) { console.error('Error rejecting plan:', e); }
  }, []);

  const editPlan = useCallback(async (text: string) => {
    const { pendingPlanAgentId: planAgent } = useUiStore.getState();
    const { selectedProjectRoot: root } = useProjectStore.getState();
    if (!planAgent || !root) return;
    try {
      const res = await fetch('/api/plan/edit', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: root, agent_id: planAgent, text }),
      });
      if (res.ok) useUiStore.getState().setPendingPlan((prev) => prev ? { ...prev, plan_text: text } : prev);
    } catch (e) { console.error('Error editing plan:', e); }
  }, []);

  const copyChat = useCallback(async () => {
    try {
      const { selectedProjectRoot: root, activeSessionId: sid } = useProjectStore.getState();
      const agent = useAgentStore.getState().selectedAgent;
      const msgs = useChatStore.getState().displayMessages;
      const headerLines = [
        'Linggen Agent Chat Export', `Project: ${root || '(none)'}`,
        `Session: ${sid || 'default'}`, `Agent: ${agent}`,
        `ExportedAt: ${new Date().toISOString()}`, '',
      ];
      const body = msgs.map((m) => {
        const from = m.from || m.role;
        const to = m.to ? ` → ${m.to}` : '';
        const lines: string[] = [`[${m.timestamp}] ${from}${to}`];
        if (m.subagentTree && m.subagentTree.length > 0) {
          for (const sa of m.subagentTree) {
            const stats = [];
            if (sa.toolCount > 0) stats.push(`${sa.toolCount} tool uses`);
            if (sa.contextTokens > 0) stats.push(`${(sa.contextTokens / 1000).toFixed(1)}k tokens`);
            lines.push(`  [subagent:${sa.subagentId}] ${sa.task}${stats.length ? ` (${stats.join(', ')})` : ''} — ${sa.status}`);
          }
        }
        const entries = Array.isArray(m.activityEntries) ? m.activityEntries : [];
        if (entries.length > 0) { for (const entry of entries) lines.push(`  > ${entry}`); }
        else if (m.activitySummary) { lines.push(`  > ${m.activitySummary}`); }
        if (m.text) lines.push(m.text);
        return lines.join('\n') + '\n';
      }).join('\n');
      await navigator.clipboard.writeText(headerLines.join('\n') + body);
      useUiStore.getState().setCopyChatStatus('copied');
      window.setTimeout(() => useUiStore.getState().setCopyChatStatus('idle'), 1200);
    } catch (e) {
      console.error('Failed to copy chat', e);
      useUiStore.getState().setCopyChatStatus('error');
      window.setTimeout(() => useUiStore.getState().setCopyChatStatus('idle'), 1600);
    }
  }, []);

  const readFile = useCallback(async (path: string, projectRootOverride?: string) => {
    const root = projectRootOverride || useProjectStore.getState().selectedProjectRoot;
    if (!root) return;
    try {
      const resp = await fetch(`/api/file?project_root=${encodeURIComponent(root)}&path=${encodeURIComponent(path)}`);
      const data = await resp.json();
      useUiStore.getState().setSelectedFileContent(data.content);
      useUiStore.getState().setSelectedFilePath(path);
    } catch (e) { console.error('Error reading file:', e); }
  }, []);

  const selectAgentPathFromTree = useCallback((projectRoot: string, path: string) => {
    if (projectRoot !== useProjectStore.getState().selectedProjectRoot) useProjectStore.getState().setSelectedProjectRoot(projectRoot);
    readFile(path, projectRoot);
  }, [readFile]);

  const switchModel = useCallback(async (modelId: string) => {
    try {
      const resp = await fetch('/api/config');
      if (resp.ok) {
        const config = await resp.json();
        const newDefaults = [modelId];
        const updated = { ...config, routing: { ...config.routing, default_models: newDefaults } };
        const saveResp = await fetch('/api/config', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(updated) });
        if (saveResp.ok) {
          useAgentStore.setState({ defaultModels: newDefaults });
          useUiStore.getState().setModelPickerOpen(false);
          useUiStore.getState().setOverlay(`Switched to: \`${modelId}\``);
        }
      }
    } catch (e) { useUiStore.getState().setOverlay(`Error switching model: ${e}`); useUiStore.getState().setModelPickerOpen(false); }
  }, []);

  // --- Render ---
  return (
    <>
    <ToastContainer />
    {!isCompact && currentPage === 'mission-editor' && (
      <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden">
        <MissionEditor
          editing={editingMission}
          projects={projects}
          onSave={() => { uiStore.bumpMissionRefreshKey(); uiStore.closeMissionEditor(); }}
          onCancel={() => uiStore.closeMissionEditor()}
          onViewAgent={() => {}}
        />
      </div>
    )}
    {!isCompact && currentPage === 'settings' && (
      <SettingsPage
        onBack={() => {
          uiStore.setCurrentPage('main');
          uiStore.setInitialSettingsTab(undefined);
          agentStore.fetchModels();
          agentStore.fetchDefaultModels();
          agentStore.fetchOllamaStatus();
        }}
        projectRoot={selectedProjectRoot}
        initialTab={uiStore.initialSettingsTab}
        missionAgents={agents}
      />
    )}
    <div className={`flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden${currentPage !== 'main' ? ' hidden' : ''}`}>
      {/* Header */}
      {!isCompact && (
        <HeaderBar
          copyChat={copyChat}
          copyChatStatus={copyChatStatus}
          clearChat={clearChat}
          isRunning={isRunning}
          onOpenSettings={() => uiStore.setCurrentPage('settings')}
        />
      )}

      {/* Compact toolbar */}
      {isCompact && (
        <div className="flex items-center gap-1.5 px-2 py-1 border-b border-slate-200 dark:border-white/10 bg-white dark:bg-[#0f0f0f] flex-shrink-0">
          <select value={selectedAgent} onChange={(e) => agentStore.setSelectedAgent(e.target.value)}
            className="text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 text-slate-700 dark:text-slate-300 outline-none max-w-[5rem]">
            {agents.map((a) => <option key={a.name} value={a.name}>{a.name}</option>)}
          </select>
          <select value={activeSessionId || ''} onChange={(e) => { projectStore.setActiveSessionId(e.target.value || null); projectStore.setIsMissionSession(false); }}
            className="text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 text-slate-700 dark:text-slate-300 outline-none flex-1 min-w-0 truncate">
            {sessions.length === 0 && <option value="">No sessions</option>}
            {sessions.map((s) => <option key={s.id} value={s.id}>{s.title || s.id.slice(0, 8)}</option>)}
          </select>
          <button onClick={() => projectStore.createSession()} title="New chat session"
            className="p-0.5 rounded hover:bg-slate-200 dark:hover:bg-white/10 text-slate-500 hover:text-slate-700 dark:hover:text-slate-300 transition-colors flex-shrink-0">
            <Plus size={14} />
          </button>
          <span className={`text-[10px] flex-shrink-0 ${isRunning ? 'text-green-500' : 'text-slate-400'}`}>
            {isRunning ? 'Running' : 'Idle'}
          </span>
        </div>
      )}

      {/* Main Layout */}
      <div className="flex-1 flex overflow-hidden">

        {/* Left sidebar */}
        {!isCompact && (
        <div className="w-72 border-r border-slate-200 dark:border-white/5 flex flex-col bg-white dark:bg-[#0f0f0f] h-full">
          {/* Tab switcher */}
          <div className="flex border-b border-slate-200 dark:border-white/5">
            <button onClick={() => uiStore.setSidebarTab('projects')}
              className={cn('flex-1 py-2 text-[11px] font-bold uppercase tracking-wider transition-colors',
                sidebarTab === 'projects' ? 'text-blue-600 dark:text-blue-400 border-b-2 border-blue-500' : 'text-slate-400 hover:text-slate-600 dark:hover:text-slate-300')}>
              Projects
            </button>
            <button onClick={() => uiStore.setSidebarTab('missions')}
              className={cn('flex-1 py-2 text-[11px] font-bold uppercase tracking-wider transition-colors',
                sidebarTab === 'missions' ? 'text-blue-600 dark:text-blue-400 border-b-2 border-blue-500' : 'text-slate-400 hover:text-slate-600 dark:hover:text-slate-300')}>
              Missions
            </button>
          </div>

          {sidebarTab === 'projects' && (<>
          <SessionNav
            projects={projects}
            selectedProjectRoot={selectedProjectRoot}
            setSelectedProjectRoot={projectStore.setSelectedProjectRoot}
            sessions={sessions.filter((s) => !s.title.startsWith('Mission:'))}
            activeSessionId={activeSessionId}
            setActiveSessionId={(id) => { projectStore.setActiveSessionId(id); projectStore.setIsMissionSession(false); }}
            createSession={() => projectStore.createSession()}
            removeSession={(id) => projectStore.removeSession(id)}
            renameSession={(id, title) => projectStore.renameSession(id, title)}
            sessionCountsByProject={projectStore.sessionCountsByProject}
            treesByProject={agentTreesByProject}
            onSelectPath={selectAgentPathFromTree}
            pickFolder={() => projectStore.pickFolder()}
            removeProject={(path) => projectStore.removeProject(path)}
          />
          <div className="border-t border-slate-200 dark:border-white/5">
            <CollapsibleCard title="AGENTS" icon={<Bot size={12} />} iconColor="text-blue-500" badge={`${mainAgents.length}`} defaultOpen
              headerAction={
                <button onClick={() => uiStore.setShowAgentSpecEditor(true)} disabled={!selectedProjectRoot}
                  title="Edit agent markdown specs" className="p-1 rounded hover:bg-slate-200 dark:hover:bg-white/10 text-slate-400 hover:text-slate-600 dark:hover:text-slate-300 disabled:opacity-30 transition-colors">
                  <FilePenLine size={12} />
                </button>
              }>
              <AgentsCard agents={mainAgents} workspaceState={chatStore.workspaceState} isRunning={isRunning}
                selectedAgent={selectedAgent} setSelectedAgent={agentStore.setSelectedAgent}
                agentStatus={agentStatus} agentStatusText={agentStatusText} agentWork={agentWork}
                agentRunSummary={agentRunSummary} agentContext={agentContext} projectRoot={selectedProjectRoot} />
            </CollapsibleCard>
          </div>
          </>)}

          {sidebarTab === 'missions' && (
          <MissionSessionNav
            activeSessionId={activeSessionId}
            setActiveSessionId={(id, missionId) => {
              useProjectStore.setState({ activeSessionId: id, isMissionSession: true, activeMissionId: missionId ?? null });
              if (id) window.localStorage.setItem('linggen:active-session', id);
            }}
            projects={projects}
            onCreateMission={() => uiStore.openMissionEditor(null)}
            onEditMission={(mission) => uiStore.openMissionEditor(mission)}
            refreshKey={missionRefreshKey}
          />
          )}
        </div>
        )}

        {/* Center: Chat */}
        <main className={`flex-1 flex flex-col overflow-hidden bg-slate-100/40 dark:bg-[#0a0a0a] min-h-0${isCompact ? ' p-0' : ''}`}>
          <div className={`flex-1 min-h-0${isCompact ? '' : ' p-2'}`}>
            <ChatPanel
              chatMessages={displayMessages}
              queuedMessages={queuedMessages}
              chatEndRef={chatEndRef}
              projectRoot={selectedProjectRoot}
              selectedAgent={selectedAgent}
              setSelectedAgent={agentStore.setSelectedAgent}
              skills={skills}
              agents={agents}
              mainAgents={mainAgents}
              subagents={subagents}
              mainRunIds={mainRunIds}
              subagentRunIds={subagentRunIds}
              runningMainRunIds={runningMainRunIds}
              runningSubagentRunIds={runningSubagentRunIds}
              mainRunHistory={mainRunHistory}
              subagentRunHistory={subagentRunHistory}
              cancellingRunIds={cancellingRunIds}
              onCancelRun={(id) => agentStore.cancelAgentRun(id)}
              onCancelAgentRun={(id) => agentStore.cancelAgentRun(id)}
              isRunning={isRunning}
              onSendMessage={sendChatMessage}
              activePlan={activePlan}
              pendingPlan={pendingPlan}
              pendingPlanAgentId={pendingPlanAgentId}
              agentContext={agentContext}
              onApprovePlan={approvePlan}
              onRejectPlan={rejectPlan}
              onEditPlan={editPlan}
              pendingAskUser={pendingAskUser}
              onRespondToAskUser={respondToAskUser}
              verboseMode={verboseMode}
              agentStatus={agentStatus}
              overlay={overlay}
              onDismissOverlay={() => { uiStore.setOverlay(null); uiStore.setModelPickerOpen(false); }}
              modelPickerOpen={modelPickerOpen}
              models={models}
              defaultModels={defaultModels}
              tokensPerSec={tokensPerSec}
              onSwitchModel={switchModel}
            />
          </div>
        </main>

        {/* Right sidebar */}
        {!isCompact && (
        <aside className="w-72 border-l border-slate-200 dark:border-white/5 flex flex-col bg-slate-100/40 dark:bg-[#0a0a0a] p-3 gap-3 overflow-y-auto">
          <CollapsibleCard title="MODELS" icon={<Sparkles size={12} />} iconColor="text-purple-500" badge={`${models.length}`} defaultOpen
            headerAction={
              <button onClick={() => uiStore.openSettings('models')}
                className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors text-slate-400 hover:text-blue-500" title="Manage Models">
                <Settings size={12} />
              </button>
            }>
            <ModelsCard models={models} agents={mainAgents} ollamaStatus={ollamaStatus} chatMessages={chatMessages}
              tokensPerSec={tokensPerSec} activeModelId={activeModelId} agentContext={agentContext}
              defaultModels={defaultModels} onToggleDefault={agentStore.toggleDefaultModel} sessionTokens={sessionTokens} />
          </CollapsibleCard>
          <CollapsibleCard title="SKILLS" icon={<Zap size={12} />} iconColor="text-amber-500" badge={`${skills.length} loaded`} defaultOpen
            headerAction={
              <div className="flex items-center gap-0.5">
                <button onClick={() => agentStore.reloadSkills()} disabled={reloadingSkills}
                  className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors text-slate-400 hover:text-blue-500 disabled:opacity-50" title="Reload skills from disk">
                  <RefreshCw size={12} className={reloadingSkills ? 'animate-spin' : ''} />
                </button>
                <button onClick={() => uiStore.openSettings('skills')}
                  className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors text-slate-400 hover:text-blue-500" title="Manage Skills">
                  <Settings size={12} />
                </button>
              </div>
            }>
            <SkillsCard skills={skills} projectRoot={selectedProjectRoot} onClickSkill={(skill) => {
              if (skill.app) {
                if (skill.app.launcher === 'web') window.open(`/apps/${skill.name}/${skill.app.entry}`, '_blank');
                else if (skill.app.launcher === 'url') window.open(skill.app.entry, '_blank');
                else sendChatMessage(`/${skill.name}`);
              } else sendChatMessage(`/${skill.name}`);
            }} />
          </CollapsibleCard>
        </aside>
        )}
      </div>

      <FilePreview selectedFilePath={selectedFilePath} selectedFileContent={selectedFileContent} onClose={() => uiStore.closeFilePreview()} />
      <AgentSpecEditorModal open={showAgentSpecEditor} projectRoot={selectedProjectRoot}
        onClose={() => uiStore.setShowAgentSpecEditor(false)}
        onChanged={() => { agentStore.fetchAgents(selectedProjectRoot); chatStore.fetchWorkspaceState(); projectStore.fetchAllAgentTrees(); }} />

      {openApp && <AppPanel app={openApp} onClose={() => uiStore.setOpenApp(null)} />}

      <style>{`
        .custom-scrollbar { scrollbar-gutter: stable; }
        .custom-scrollbar::-webkit-scrollbar { width: 8px; }
        .custom-scrollbar::-webkit-scrollbar-track { background: rgba(0, 0, 0, 0.04); }
        .custom-scrollbar::-webkit-scrollbar-thumb { background: rgba(59, 130, 246, 0.45); border-radius: 10px; }
        .custom-scrollbar::-webkit-scrollbar-thumb:hover { background: rgba(59, 130, 246, 0.7); }
        html[data-compact] { --vsc-bg: #1e1e1e; --vsc-sidebar: #252526; --vsc-input: #3c3c3c; --vsc-border: #3c3c3c; --vsc-fg: #cccccc; --vsc-fg-muted: #858585; --vsc-accent: #0e639c; color-scheme: dark; }
        html[data-compact] .dark\\:bg-\\[\\#0a0a0a\\] { background-color: var(--vsc-bg) !important; }
        html[data-compact] .dark\\:bg-\\[\\#0f0f0f\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:bg-white\\/\\[0\\.02\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:bg-white\\/5 { background-color: rgba(255,255,255,0.03) !important; }
        html[data-compact] .dark\\:bg-black\\/20 { background-color: var(--vsc-input) !important; }
        html[data-compact] .dark\\:bg-black\\/30 { background-color: var(--vsc-input) !important; }
        html[data-compact] .dark\\:border-white\\/5, html[data-compact] .dark\\:border-white\\/10 { border-color: var(--vsc-border) !important; }
        html[data-compact] section { border-radius: 0 !important; border: none !important; }
        html[data-compact] .dark\\:text-slate-200 { color: var(--vsc-fg) !important; }
        html[data-compact] .dark\\:text-slate-300 { color: #d4d4d4 !important; }
        html[data-compact] .dark\\:text-slate-400 { color: #969696 !important; }
        html[data-compact] .text-slate-500 { color: #969696 !important; }
        html[data-compact] .text-slate-400 { color: #969696 !important; }
        html[data-compact] .text-slate-600 { color: #b0b0b0 !important; }
        html[data-compact] select, html[data-compact] input, html[data-compact] textarea { color: var(--vsc-fg) !important; background-color: var(--vsc-input) !important; border-color: var(--vsc-border) !important; font-size: 13px !important; }
        html[data-compact] ::placeholder { color: var(--vsc-fg-muted) !important; opacity: 1 !important; }
        html[data-compact] body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; font-size: 13px; color: var(--vsc-fg); }
        html[data-compact] .custom-scrollbar::-webkit-scrollbar-thumb { background: rgba(121,121,121,0.4); border-radius: 0; }
        html[data-compact] .custom-scrollbar::-webkit-scrollbar-thumb:hover { background: rgba(121,121,121,0.7); }
        html[data-compact] .custom-scrollbar::-webkit-scrollbar-track { background: transparent; }
        html[data-compact] .dark\\:bg-\\[\\#141414\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:border-amber-500\\/20, html[data-compact] .dark\\:border-amber-500\\/10 { border-color: var(--vsc-border) !important; }
        html[data-compact] .dark\\:bg-amber-500\\/5 { background-color: rgba(255,255,255,0.03) !important; }
        html[data-compact] .dark\\:bg-amber-500\\/10 { background-color: rgba(255,255,255,0.05) !important; }
        html[data-compact] .dark\\:text-amber-400 { color: #cca700 !important; }
        html[data-compact] .dark\\:text-amber-300 { color: #ddb700 !important; }
        html[data-compact] .dark\\:text-amber-300\\/80 { color: rgba(221,183,0,0.8) !important; }
        html[data-compact] .dark\\:hover\\:border-amber-500\\/40:hover { border-color: #cca700 !important; }
        html[data-compact] .dark\\:hover\\:bg-amber-500\\/5:hover { background-color: rgba(255,255,255,0.05) !important; }
        html[data-compact] .dark\\:border-amber-500\\/30 { border-color: rgba(204,167,0,0.4) !important; }
        html[data-compact] .dark\\:border-blue-500\\/20, html[data-compact] .dark\\:border-blue-500\\/10 { border-color: var(--vsc-border) !important; }
        html[data-compact] .dark\\:bg-blue-500\\/5 { background-color: rgba(255,255,255,0.03) !important; }
        html[data-compact] .dark\\:text-blue-400 { color: #569cd6 !important; }
        html[data-compact] .dark\\:hover\\:border-blue-500\\/40:hover { border-color: #569cd6 !important; }
        html[data-compact] .dark\\:bg-blue-500 { background-color: var(--vsc-accent) !important; border-color: var(--vsc-accent) !important; }
        html[data-compact] .dark\\:hover\\:bg-blue-600:hover { background-color: #1177bb !important; }
        html[data-compact] .dark\\:bg-white\\/5 { background-color: var(--vsc-input) !important; border-color: var(--vsc-border) !important; }
        html[data-compact] .dark\\:hover\\:bg-white\\/10:hover { background-color: rgba(255,255,255,0.08) !important; }
        html[data-compact] .dark\\:hover\\:border-red-500\\/40:hover { border-color: #f14c4c !important; }
        html[data-compact] .dark\\:hover\\:text-red-400:hover { color: #f14c4c !important; }
        html[data-compact] .dark\\:hover\\:border-slate-400\\/30:hover { border-color: var(--vsc-fg-muted) !important; }
        html[data-compact] .dark\\:hover\\:text-slate-400:hover { color: #b0b0b0 !important; }
        html[data-compact] .dark\\:hover\\:text-slate-300:hover { color: #d4d4d4 !important; }
        html[data-compact] .dark\\:hover\\:text-amber-400:hover { color: #cca700 !important; }
        html[data-compact] .dark\\:hover\\:border-amber-500\\/30:hover { border-color: rgba(204,167,0,0.4) !important; }
        html[data-compact] .dark\\:bg-amber-600 { background-color: #b8860b !important; }
        html[data-compact] .dark\\:hover\\:bg-amber-700:hover { background-color: #996f0a !important; }
        html[data-compact] section { border-radius: 2px !important; border: 1px solid var(--vsc-border) !important; }
        html[data-compact] .rounded-xl { border-radius: 2px !important; }
        html[data-compact] .rounded-lg { border-radius: 2px !important; }
        html[data-compact] .rounded-md { border-radius: 2px !important; }
        html[data-compact] .shadow-sm { box-shadow: none !important; }
        html[data-compact] .shadow-xl { box-shadow: 0 2px 8px rgba(0,0,0,0.36) !important; }
        html[data-compact] .dark\\:text-green-300 { color: #89d185 !important; }
        html[data-compact] .dark\\:text-blue-300 { color: #569cd6 !important; }
        html[data-compact] .dark\\:text-amber-300 { color: #cca700 !important; }
        html[data-compact] .dark\\:text-indigo-300 { color: #b4a0ff !important; }
        html[data-compact] .dark\\:text-slate-600 { color: #5a5a5a !important; }
        html[data-compact] .dark\\:text-slate-500 { color: #6e6e6e !important; }
        html[data-compact] .dark\\:focus\\:border-amber-500\\/40:focus { border-color: var(--vsc-accent) !important; }
        html[data-compact] .dark\\:focus\\:border-blue-500\\/40:focus { border-color: var(--vsc-accent) !important; }
        html[data-compact] .dark\\:bg-white\\/\\[0\\.02\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:hover\\:bg-white\\/5:hover { background-color: rgba(255,255,255,0.06) !important; }
        html[data-compact] .markdown-body { color: var(--vsc-fg, #ccc) !important; }
        html[data-compact] .markdown-body code { background: rgba(255,255,255,0.1) !important; color: #d4d4d4 !important; }
        html[data-compact] .markdown-body pre { background: rgba(255,255,255,0.06) !important; }
        html[data-compact] .markdown-body pre code { color: #c9d1d9 !important; }
        html[data-compact] .markdown-body th, html[data-compact] .markdown-body td { border-color: rgba(255,255,255,0.15) !important; color: #d4d4d4 !important; }
        html[data-compact] .markdown-body th { background: rgba(255,255,255,0.06) !important; }
        html[data-compact] .markdown-body hr { border-top-color: rgba(255,255,255,0.12) !important; }
        html[data-compact] .markdown-body blockquote { border-left-color: rgba(59,130,246,0.5) !important; color: #b4becd !important; }
        html[data-compact] .markdown-body h1, html[data-compact] .markdown-body h2, html[data-compact] .markdown-body h3, html[data-compact] .markdown-body h4 { color: #e0e0e0 !important; }
        html[data-compact] .hljs { color: #c9d1d9 !important; background: transparent !important; }
        html[data-compact] .hljs-doctag, html[data-compact] .hljs-keyword, html[data-compact] .hljs-template-tag, html[data-compact] .hljs-template-variable, html[data-compact] .hljs-type, html[data-compact] .hljs-variable.language_ { color: #ff7b72 !important; }
        html[data-compact] .hljs-title, html[data-compact] .hljs-title.class_, html[data-compact] .hljs-title.function_ { color: #d2a8ff !important; }
        html[data-compact] .hljs-attr, html[data-compact] .hljs-attribute, html[data-compact] .hljs-literal, html[data-compact] .hljs-meta, html[data-compact] .hljs-number, html[data-compact] .hljs-operator, html[data-compact] .hljs-variable, html[data-compact] .hljs-selector-attr, html[data-compact] .hljs-selector-class, html[data-compact] .hljs-selector-id { color: #79c0ff !important; }
        html[data-compact] .hljs-regexp, html[data-compact] .hljs-string { color: #a5d6ff !important; }
        html[data-compact] .hljs-built_in, html[data-compact] .hljs-symbol { color: #ffa657 !important; }
        html[data-compact] .hljs-comment, html[data-compact] .hljs-code, html[data-compact] .hljs-formula { color: #8b949e !important; }
        html[data-compact] .hljs-name, html[data-compact] .hljs-quote, html[data-compact] .hljs-selector-tag, html[data-compact] .hljs-selector-pseudo { color: #7ee787 !important; }
        html[data-compact] .hljs-subst { color: #c9d1d9 !important; }
        html[data-compact] .hljs-addition { color: #aff5b4 !important; background-color: #033a16 !important; }
        html[data-compact] .hljs-deletion { color: #ffdcd7 !important; background-color: #67060c !important; }
      `}</style>
    </div>
    </>
  );
};

export default App;
