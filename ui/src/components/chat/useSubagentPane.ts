// The subagent pane beside the chat. It opens when a subagent or background
// task starts running, or a subagent asks the person something, and closes
// itself a few seconds after the last one ends — unless a subagent still
// waits on an answer or the person is reading an item's detail. Ended ones
// alone never open it; the chat's tasks button does.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { BackgroundTask, ChatMessage, PendingAskUser, SubagentTreeEntry } from '../../types';
import { normalizeAgentKey } from '../../lib/messageUtils';
import { paneItems } from '../../lib/paneItems.mts';

const CLOSE_AFTER_MS = 5_000;

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
  const askUserBelongsToSubagent = useMemo(
    () => askIsForSubagent(pendingAskUser, entries, selectedAgent, members),
    [pendingAskUser, entries, selectedAgent, members],
  );
  const [paneVisible, setPaneVisible] = useState(false);
  const [reading, setReading] = useState(false);

  // What runs, counted the way the pane lists it: a background subagent once.
  const items = useMemo(() => paneItems(entries, tasks), [entries, tasks]);
  const runningKeys = items.filter((i) => i.running).map((i) => i.key);
  const runningCount = runningKeys.length;

  // An item that was not running before opens the pane, once.
  const seenRunning = useRef<Set<string>>(new Set());
  const runningSig = runningKeys.join('|');
  useEffect(() => {
    const keys = runningSig ? runningSig.split('|') : [];
    if (keys.some((k) => !seenRunning.current.has(k))) setPaneVisible(true);
    seenRunning.current = new Set(keys);
  }, [runningSig]);

  useEffect(() => {
    if (askUserBelongsToSubagent) setPaneVisible(true);
  }, [askUserBelongsToSubagent]);

  // Nothing runs any more: close after a moment to read the last result.
  useEffect(() => {
    if (!paneVisible || runningCount > 0 || askUserBelongsToSubagent || reading) return;
    const t = setTimeout(() => setPaneVisible(false), CLOSE_AFTER_MS);
    return () => clearTimeout(t);
  }, [paneVisible, runningCount, askUserBelongsToSubagent, reading]);

  return {
    askUserBelongsToSubagent,
    runningCount,
    hasItems: items.length > 0,
    paneVisible,
    setReading,
    closePane: () => setPaneVisible(false),
    openPane: () => setPaneVisible(true),
  };
}
