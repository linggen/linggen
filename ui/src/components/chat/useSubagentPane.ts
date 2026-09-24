// The subagent pane beside the chat: shown while any message has a
// subagent tree, forced open while one runs or asks the user something,
// and auto-collapsed 10 minutes after the last one finishes.
import { useEffect, useMemo, useState } from 'react';
import type { ChatMessage, PendingAskUser, SubagentTreeEntry } from '../../types';
import { normalizeAgentKey } from './utils/message';

const COLLAPSE_AFTER_MS = 600_000;

/** Is the pending AskUser for a subagent (the pane owns the widget) or for
 *  the main chat? */
function askIsForSubagent(ask: PendingAskUser | null | undefined, entries: SubagentTreeEntry[], mainAgent: string): boolean {
  if (!ask) return false;
  const id = normalizeAgentKey(ask.agentId);
  if (!id) return false;
  // A tree match works once SubagentSpawned has filled the parent's tree.
  if (entries.some((e) => normalizeAgentKey(e.subagentId) === id || normalizeAgentKey(e.agentName) === id)) return true;
  // Otherwise any agent but the session's main one is a subagent (the
  // per-turn encoder is "ling-mem") — covers a spawn event not yet in.
  return id !== normalizeAgentKey(mainAgent);
}

export function useSubagentPane(messages: ChatMessage[], pendingAskUser: PendingAskUser | null | undefined, selectedAgent: string) {
  const entries = useMemo(() => messages.flatMap((m) => m.subagentTree ?? []), [messages]);
  const anyRunning = entries.some((e) => e.status === 'running');
  const askUserBelongsToSubagent = useMemo(
    () => askIsForSubagent(pendingAskUser, entries, selectedAgent),
    [pendingAskUser, entries, selectedAgent],
  );
  const [paneVisible, setPaneVisible] = useState(false);
  useEffect(() => {
    if (entries.length === 0) {
      setPaneVisible(false);
      return;
    }
    setPaneVisible(true);
    // Running or waiting on the user: stay open. Else give ample time to read
    // the subagent's chat before collapsing; a new spawn cancels it.
    if (anyRunning || askUserBelongsToSubagent) return;
    const t = setTimeout(() => setPaneVisible(false), COLLAPSE_AFTER_MS);
    return () => clearTimeout(t);
  }, [anyRunning, entries.length, askUserBelongsToSubagent]);

  return { askUserBelongsToSubagent, paneVisible, closePane: () => setPaneVisible(false) };
}
