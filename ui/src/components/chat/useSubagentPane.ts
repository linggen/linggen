// The subagent pane beside the chat: shown while any message has a
// subagent tree, forced open while one runs or asks the user something,
// and auto-collapsed 10 minutes after the last one finishes. A background
// task opens it when it starts but never holds it open: it can run for
// hours, and the chat shows how many run while the pane is closed.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { BackgroundTask, ChatMessage, PendingAskUser, SubagentTreeEntry } from '../../types';
import { normalizeAgentKey } from '../../lib/messageUtils';

const COLLAPSE_AFTER_MS = 600_000;

/** Is the pending AskUser for a subagent (the pane owns the widget) or for
 *  the main chat? Any member of the session asks in the main chat — a
 *  permission prompt raised on Yinyue's turn is the person's to see there. */
function askIsForSubagent(ask: PendingAskUser | null | undefined, entries: SubagentTreeEntry[], mainAgent: string, members: readonly string[]): boolean {
  if (!ask) return false;
  const id = normalizeAgentKey(ask.agentId);
  if (!id) return false;
  if (members.some((m) => normalizeAgentKey(m) === id)) return false;
  // A tree match works once SubagentSpawned has filled the parent's tree.
  if (entries.some((e) => normalizeAgentKey(e.subagentId) === id || normalizeAgentKey(e.agentName) === id)) return true;
  // Otherwise any agent but the session's main one is a subagent (the
  // per-turn encoder is "ling-mem") — covers a spawn event not yet in.
  return id !== normalizeAgentKey(mainAgent);
}

export function useSubagentPane(messages: ChatMessage[], pendingAskUser: PendingAskUser | null | undefined, selectedAgent: string, members: readonly string[] = [], tasks: BackgroundTask[] = []) {
  const entries = useMemo(() => messages.flatMap((m) => m.subagentTree ?? []), [messages]);
  const anyRunning = entries.some((e) => e.status === 'running');
  const askUserBelongsToSubagent = useMemo(
    () => askIsForSubagent(pendingAskUser, entries, selectedAgent, members),
    [pendingAskUser, entries, selectedAgent, members],
  );
  const [paneVisible, setPaneVisible] = useState(false);
  useEffect(() => {
    if (entries.length === 0) {
      // Only background tasks to show: the person opens and closes it.
      if (tasks.length === 0) setPaneVisible(false);
      return;
    }
    setPaneVisible(true);
    // Running or waiting on the user: stay open. Else give ample time to read
    // the subagent's chat before collapsing; a new spawn cancels it.
    if (anyRunning || askUserBelongsToSubagent) return;
    const t = setTimeout(() => setPaneVisible(false), COLLAPSE_AFTER_MS);
    return () => clearTimeout(t);
  }, [anyRunning, entries.length, askUserBelongsToSubagent, tasks.length]);

  // A task that was not running before opens the pane, once.
  const seenRunning = useRef<Set<string>>(new Set());
  useEffect(() => {
    const running = tasks.filter((t) => t.status === 'running').map((t) => t.id);
    if (running.some((id) => !seenRunning.current.has(id))) setPaneVisible(true);
    seenRunning.current = new Set(running);
  }, [tasks]);

  return {
    askUserBelongsToSubagent,
    paneVisible,
    closePane: () => setPaneVisible(false),
    openPane: () => setPaneVisible(true),
  };
}
