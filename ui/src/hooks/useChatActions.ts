/**
 * Chat action callbacks — sendChatMessage, clearChat, plan actions, copyChat.
 * All state is read via getState() — only scrollToBottom and projectRoot are injected.
 */
import { useCallback, useEffect, useRef } from 'react';
import { useProjectStore } from '../stores/projectStore';
import { useAgentStore } from '../stores/agentStore';
import { useChatStore } from '../stores/chatStore';
import { useUiStore } from '../stores/uiStore';
import { getTransport } from '../lib/transport';

/** Resolve the effective project root: explicit override > store value. */
function getProjectRoot(override?: string | null): string {
  return override || useProjectStore.getState().selectedProjectRoot;
}

export function useChatActions(
  scrollToBottom: () => void,
  runningMainRunIds: Record<string, string>,
  /** When provided, API calls use this instead of selectedProjectRoot from the store. */
  projectRootOverride?: string | null,
) {
  // Use a ref so callbacks don't need to be recreated when the override changes
  const projectRootRef = useRef(projectRootOverride);
  useEffect(() => { projectRootRef.current = projectRootOverride; }, [projectRootOverride]);

  const clearChat = useCallback(async () => {
    const root = getProjectRoot(projectRootRef.current);
    const { activeSessionId: sid } = useProjectStore.getState();

    const selectedAgent = useAgentStore.getState().selectedAgent;
    const runId = runningMainRunIds[selectedAgent];
    if (runId) {
      useAgentStore.getState().cancelAgentRun(runId);
    }

    useChatStore.getState().clear();
    const ui = useUiStore.getState();
    ui.setQueuedMessages([]);
    ui.setActivePlan(null);
    ui.setPendingPlan(null);
    ui.setPendingPlanAgentId(null);
    ui.setPendingAskUser(null);
    try {
      await getTransport().sendClear(root, sid);
      useChatStore.getState().clear();
    } catch (e) { console.error('Error clearing chat:', e); }
  }, [runningMainRunIds]);

  const sendChatMessage = useCallback(async (userMessage: string, targetAgent?: string, images?: string[]) => {
    if (!userMessage.trim() && !(images && images.length > 0)) return;
    const root = getProjectRoot(projectRootRef.current);
    const { activeSessionId: sid } = useProjectStore.getState();
    const agent = useAgentStore.getState().selectedAgent;
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
        ...(images && images.length > 0 ? { images, imageCount: images.length } : {}),
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
          body: JSON.stringify({ project_root: root, command: cmd, session_id: sid }),
        });
        const resultTs = new Date();
        if (!resp.ok) {
          const errText = await resp.text().catch(() => `HTTP ${resp.status}`);
          chat.addMessage({
            role: 'assistant', from: 'system', to: 'user',
            text: `Error: ${errText}`, isError: true,
            timestamp: resultTs.toLocaleTimeString(), timestampMs: resultTs.getTime(), isGenerating: false,
          });
        } else {
          const data = await resp.json();
          const output = [data.stdout, data.stderr].filter(Boolean).join('\n').trim();
          const exitInfo = data.exit_code !== 0 ? `\n\n(exit code ${data.exit_code})` : '';
          chat.addMessage({
            role: 'assistant', from: 'system', to: 'user',
            text: output ? `\`\`\`\n${output}\n\`\`\`${exitInfo}` : `(no output)${exitInfo}`,
            timestamp: resultTs.toLocaleTimeString(), timestampMs: resultTs.getTime(), isGenerating: false,
          });
        }
        scrollToBottom();
      } catch (e) {
        console.error('Bash error:', e);
        const errTs = new Date();
        chat.addMessage({
          role: 'assistant', from: 'system', to: 'user',
          text: `Error: ${e instanceof Error ? e.message : String(e)}`, isError: true,
          timestamp: errTs.toLocaleTimeString(), timestampMs: errTs.getTime(), isGenerating: false,
        });
        scrollToBottom();
      }
      return;
    }

    if (trimmed === '/compact' || trimmed.startsWith('/compact ')) {
      const focus = trimmed.slice('/compact'.length).trim() || undefined;
      const { setAgentStatus, setAgentStatusText } = useAgentStore.getState();
      setAgentStatus((s) => ({ ...s, [agentToUse]: 'thinking' as const }));
      setAgentStatusText((s) => ({ ...s, [agentToUse]: 'Compacting conversation' }));
      try {
        const data = await getTransport().sendCompact(root, sid, agentToUse, focus) as any;
        const clearStatus = () => {
          setAgentStatus((s) => ({ ...s, [agentToUse]: 'idle' as const }));
          setAgentStatusText((s) => { const n = { ...s }; delete n[agentToUse]; return n; });
        };
        clearStatus();
        if (data.compacted) {
          useChatStore.getState().clear(false);
          await useChatStore.getState().fetchWorkspaceState();
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
      const { isMissionSession, activeMissionId, isSkillSession, activeSkillName } = useProjectStore.getState();
      const data = await getTransport().sendChat({
        project_root: root,
        agent_id: agentToUse,
        message: userMessage,
        session_id: sid,
        ...(isMissionSession && activeMissionId ? { mission_id: activeMissionId } : {}),
        ...(isSkillSession && activeSkillName ? { skill_name: activeSkillName } : {}),
        ...(useUiStore.getState().sessionModel ? { model_id: useUiStore.getState().sessionModel } : {}),
        ...(images && images.length > 0 ? { images } : {}),
      }) as any;
      if (data?.session_id && !sid) {
        useProjectStore.getState().setActiveSessionId(data.session_id);
        useProjectStore.getState().fetchSessions();
        if (window.parent !== window) {
          window.parent.postMessage({ type: 'linggen-skill-event', event: 'session_created', payload: { sessionId: data.session_id } }, '*');
        }
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
      await getTransport().sendAskUserResponse({
        question_id: questionId,
        answers,
        session_id: useProjectStore.getState().activeSessionId,
      });
      useUiStore.getState().setPendingAskUser(null);
    } catch (e) { console.error('Error responding to AskUser:', e); }
  }, []);

  const approvePlan = useCallback(async () => {
    const { pendingPlanAgentId: planAgent } = useUiStore.getState();
    const root = getProjectRoot(projectRootRef.current);
    const { activeSessionId: sid } = useProjectStore.getState();
    if (!planAgent || !root) return;
    try {
      await getTransport().sendPlanAction({
        type: 'approve',
        project_root: root,
        agent_id: planAgent,
        session_id: sid,
      });
      const ui = useUiStore.getState();
      // Optimistically mark plan as approved so content stays visible but buttons hide
      ui.setPendingPlan((p) => p ? { ...p, status: 'approved' } : null);
    } catch (e) { console.error('Error approving plan:', e); }
  }, []);

  const rejectPlan = useCallback(async () => {
    const { pendingPlanAgentId: planAgent } = useUiStore.getState();
    const root = getProjectRoot(projectRootRef.current);
    const { activeSessionId: sid } = useProjectStore.getState();
    if (!planAgent || !root) return;
    try {
      await getTransport().sendPlanAction({
        type: 'reject',
        project_root: root,
        agent_id: planAgent,
        session_id: sid,
      });
      const ui = useUiStore.getState();
      ui.setPendingPlan((p) => p ? { ...p, status: 'rejected' } : null);
    } catch (e) { console.error('Error rejecting plan:', e); }
  }, []);

  const editPlan = useCallback(async (text: string) => {
    const { pendingPlanAgentId: planAgent } = useUiStore.getState();
    const root = getProjectRoot(projectRootRef.current);
    const { activeSessionId: sid } = useProjectStore.getState();
    if (!planAgent || !root) return;
    try {
      await getTransport().sendPlanAction({
        type: 'edit',
        project_root: root,
        agent_id: planAgent,
        session_id: sid,
        edited_plan: text,
      });
      useUiStore.getState().setPendingPlan((prev) => prev ? { ...prev, plan_text: text } : prev);
    } catch (e) { console.error('Error editing plan:', e); }
  }, []);

  const copyChat = useCallback(async () => {
    try {
      const root = getProjectRoot(projectRootRef.current);
      const { activeSessionId: sid } = useProjectStore.getState();
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

  return {
    sendChatMessage,
    clearChat,
    respondToAskUser,
    approvePlan,
    rejectPlan,
    editPlan,
    copyChat,
  };
}
