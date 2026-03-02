/**
 * Pure utility functions for chat message processing.
 * Extracted from App.tsx — no React dependencies, no side effects.
 */
import type {
  ChatMessage,
  ContentBlock,
  SubagentTreeEntry,
  AgentTreeItem,
  AgentWorkInfo,
  SubagentInfo,
} from '../types';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

export const LIVE_MESSAGE_GRACE_MS = 10_000;
export const TOKEN_RATE_WINDOW_MS = 8_000;
export const TOKEN_RATE_IDLE_RESET_MS = 10_000;

// ---------------------------------------------------------------------------
// Tool parsing helpers
// ---------------------------------------------------------------------------

const TOOL_JSON_EMBEDDED_RE = /\{"type":"tool","tool":"([^"]+)","args":\{[\s\S]*?\}\}/g;
const TOOL_RESULT_LINE_RE = /^(Tool\s+[A-Za-z0-9_.:-]+\s*:|tool_error:|tool_not_allowed:)/i;

export const parseToolNameFromParsedPayload = (parsed: any): string | null => {
  if (!parsed || typeof parsed !== 'object') return null;
  if (parsed?.type === 'tool' && typeof parsed?.tool === 'string') {
    return parsed.tool;
  }
  if (
    typeof parsed?.type === 'string' &&
    parsed.type !== 'finalize_task' &&
    parsed.args &&
    typeof parsed.args === 'object'
  ) {
    return parsed.type;
  }
  return null;
};

export const parseToolNameFromMessage = (text: string): string | null => {
  try {
    const parsed = JSON.parse(text);
    return parseToolNameFromParsedPayload(parsed);
  } catch (_e) {
    // Non-JSON messages are ignored.
  }
  return null;
};

export const extractToolNamesFromText = (text: string): string[] => {
  const names: string[] = [];
  const direct = parseToolNameFromMessage(text.trim());
  if (direct) names.push(direct);
  const lines = text
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  for (const line of lines) {
    const n = parseToolNameFromMessage(line);
    if (n) names.push(n);
  }
  let m: RegExpExecArray | null = null;
  TOOL_JSON_EMBEDDED_RE.lastIndex = 0;
  while ((m = TOOL_JSON_EMBEDDED_RE.exec(text)) !== null) {
    if (m[1]) names.push(m[1]);
  }
  return Array.from(new Set(names));
};

export const stripToolPayloadLines = (text: string): string => {
  const withoutEmbedded = text.replace(TOOL_JSON_EMBEDDED_RE, '').trim();
  const cleaned = withoutEmbedded
    .split('\n')
    .map((line) => line.trimEnd())
    .filter((line) => {
      const t = line.trim();
      if (!t) return false;
      if (parseToolNameFromMessage(t)) return false;
      if (TOOL_RESULT_LINE_RE.test(t)) return false;
      return true;
    })
    .join('\n')
    .trim();
  return cleaned;
};

export const isToolResultMessage = (from?: string, text?: string) => {
  if (!text) return false;
  const trimmed = text.trim();
  if (!trimmed) return false;
  return TOOL_RESULT_LINE_RE.test(trimmed) || (from === 'system' && trimmed.startsWith('Tool '));
};

// ---------------------------------------------------------------------------
// Structured JSON stripping
// ---------------------------------------------------------------------------

/** Strip embedded structured JSON (plan, tool calls, finalize_task) from text,
 *  handling nested braces correctly via depth counting. */
export const stripEmbeddedStructuredJson = (text: string): string => {
  const MARKERS = ['"type":"plan"', '"type":"tool"', '"type":"finalize_task"', '"type": "plan"', '"type": "tool"'];
  let result = text;

  for (const marker of MARKERS) {
    let searchFrom = 0;
    while (true) {
      const markerIdx = result.indexOf(marker, searchFrom);
      if (markerIdx < 0) break;

      let start = -1;
      for (let i = markerIdx - 1; i >= 0; i--) {
        if (result[i] === '{') { start = i; break; }
      }
      if (start < 0) { searchFrom = markerIdx + marker.length; continue; }

      let depth = 0;
      let end = -1;
      for (let i = start; i < result.length; i++) {
        if (result[i] === '{') depth++;
        else if (result[i] === '}') {
          depth--;
          if (depth === 0) { end = i + 1; break; }
        }
      }
      if (end < 0) break;

      result = result.slice(0, start) + result.slice(end);
    }
  }

  return result.trim();
};

// ---------------------------------------------------------------------------
// Status / activity classification
// ---------------------------------------------------------------------------

export const isStatusLineText = (text: string) =>
  text === 'Thinking...' ||
  text === 'Thinking' ||
  text.startsWith('Thinking (') ||
  text === 'Model loading...' ||
  text.startsWith('Loading model:') ||
  text === 'Running' ||
  text === 'Reading file...' ||
  text.startsWith('Reading file:') ||
  text === 'Writing file...' ||
  text.startsWith('Writing file:') ||
  text === 'Running command...' ||
  text.startsWith('Running command:') ||
  text === 'Searching...' ||
  text.startsWith('Searching:') ||
  text === 'Listing files...' ||
  text.startsWith('Listing files:') ||
  text === 'Delegating...' ||
  text.startsWith('Delegating to subagent:') ||
  text === 'Calling tool...' ||
  text.startsWith('Calling tool:') ||
  text.startsWith('Used tool:');

export const activityKind = (line?: string): string => {
  const t = String(line || '').trim().toLowerCase();
  if (!t) return '';
  if (t === 'reading file...' || t.startsWith('reading file:') || t.startsWith('read file')) return 'read';
  if (t === 'writing file...' || t.startsWith('writing file:') || t.startsWith('wrote ')) return 'write';
  if (t === 'editing file...' || t.startsWith('editing file:') || t.startsWith('edited ')) return 'edit';
  if (t === 'running command...' || t.startsWith('running command:') || t.startsWith('ran command')) return 'bash';
  if (t === 'searching...' || t.startsWith('searching:') || t.startsWith('searched')) return 'grep';
  if (t === 'listing files...' || t.startsWith('listing files:') || t.startsWith('listed files')) return 'glob';
  if (t === 'delegating...' || t.startsWith('delegating to subagent:') || t.startsWith('delegated to ')) return 'task';
  if (t === 'calling tool...' || t.startsWith('calling tool:') || t.startsWith('used tool')) return 'calling_tool';
  return '';
};

export const isGenericActivityLine = (line?: string): boolean => {
  const t = String(line || '').trim().toLowerCase();
  return (
    t === 'reading file...' ||
    t === 'writing file...' ||
    t === 'editing file...' ||
    t === 'running command...' ||
    t === 'searching...' ||
    t === 'listing files...' ||
    t === 'delegating...' ||
    t === 'calling tool...'
  );
};

/** Detect "in-progress" activity lines (present continuous verb). */
export const isDoingActivityLine = (line?: string): boolean => {
  const t = String(line || '').trim().toLowerCase();
  return (
    t.startsWith('reading file') ||
    t.startsWith('writing file') ||
    t.startsWith('editing file') ||
    t.startsWith('running command') ||
    t.startsWith('searching') ||
    t.startsWith('listing files') ||
    t.startsWith('delegating') ||
    t.startsWith('calling tool')
  );
};

/** Detect "completed" activity lines (past tense verb). */
export const isDoneActivityLine = (line?: string): boolean => {
  const t = String(line || '').trim().toLowerCase();
  return (
    t.startsWith('read file') ||
    t.startsWith('wrote ') ||
    t.startsWith('edited ') ||
    t.startsWith('ran command') ||
    t.startsWith('searched:') ||
    t.startsWith('searched for ') ||
    t.startsWith('listed files') ||
    t.startsWith('delegated to ') ||
    t.startsWith('used tool')
  );
};

/** Transient status lines that should not appear as activity entries. */
export const isTransientStatus = (line: string): boolean => {
  const t = line.trim().toLowerCase();
  if (t === 'thinking' || t === 'thinking...' || t === 'model loading' || t === 'model loading...' || t === 'running') return true;
  // Match "Thinking (model-name)" and "Model loading (model-name)" patterns
  if (t.startsWith('thinking (') || t.startsWith('thinking(')) return true;
  if (t.startsWith('model loading (') || t.startsWith('model loading(')) return true;
  // "Loading model: qwen3.5:35b" — transient, replaced by Thinking once loaded
  if (t.startsWith('loading model:') || t.startsWith('loading model ')) return true;
  return false;
};

/** Extract the detail/target portion after the verb (e.g. "foo.rs" from "Reading file: foo.rs"). */
export const activityDetail = (line?: string): string => {
  const idx = String(line || '').indexOf(': ');
  return idx >= 0 ? String(line || '').slice(idx + 2).trim() : '';
};

// ---------------------------------------------------------------------------
// Activity entry management
// ---------------------------------------------------------------------------

export const summarizeActivityEntries = (entries: string[], inProgress = false): string | undefined => {
  if (entries.length === 0) return undefined;
  const tools = entries
    .filter((line) => /^Calling tool:/i.test(line))
    .map((line) => line.replace(/^Calling tool:\s*/i, '').trim())
    .filter(Boolean);
  const uniqueTools = Array.from(new Set(tools));
  const phases = entries.filter((line) => !/^Calling tool:/i.test(line));
  const normalized = entries.map((line) => line.toLowerCase());
  const readCount = normalized.filter((v) => v.startsWith('read ') || v.includes('reading file')).length;
  const searchCount = normalized.filter((v) => v.startsWith('searched for ') || v.includes('searching') || v.includes('grep')).length;
  const listCount = normalized.filter((v) => v.startsWith('listed files') || v.includes('listing files') || v.includes('glob')).length;
  if (readCount > 0 || searchCount > 0 || listCount > 0) {
    const parts: string[] = [];
    if (readCount > 0) parts.push(`${readCount} file${readCount > 1 ? 's' : ''}`);
    if (searchCount > 0) parts.push(`${searchCount} search${searchCount > 1 ? 'es' : ''}`);
    if (listCount > 0) parts.push(`${listCount} list${listCount > 1 ? 's' : ''}`);
    return `${inProgress ? 'Exploring' : 'Explored'} ${parts.join(', ')}`;
  }
  const phaseSummary =
    phases.length > 1 ? `${phases[0]} -> ${phases[phases.length - 1]}` : phases[0] || '';
  const toolSummary =
    tools.length > 0
      ? `${tools.length} tool call${tools.length > 1 ? 's' : ''}${
          uniqueTools.length > 0
            ? `: ${uniqueTools.slice(0, 3).join(', ')}${uniqueTools.length > 3 ? ', ...' : ''}`
            : ''
        }`
      : '';
  if (phaseSummary && toolSummary) return `${phaseSummary} • ${toolSummary}`;
  return toolSummary || phaseSummary;
};

export const addActivityEntry = (msg: ChatMessage, entry: string): ChatMessage => {
  const clean = entry.trim();
  if (!clean) return msg;

  if (isTransientStatus(clean)) return msg;

  const entries = msg.activityEntries ? [...msg.activityEntries] : [];
  const nextKind = activityKind(clean);

  if (entries.length === 0) {
    entries.push(clean);
  } else {
    if (nextKind && isDoneActivityLine(clean)) {
      let replaced = false;
      const nextDetail = activityDetail(clean);
      for (let i = entries.length - 1; i >= 0; i--) {
        if (activityKind(entries[i]) === nextKind && isDoingActivityLine(entries[i])) {
          const entryDetail = activityDetail(entries[i]);
          if (!nextDetail || !entryDetail || nextDetail === entryDetail) {
            entries[i] = clean;
            replaced = true;
            break;
          }
        }
      }
      if (!replaced) {
        entries.push(clean);
      }
    } else {
      const last = entries[entries.length - 1];
      if (last === clean) {
        // Exact duplicate — skip.
      } else {
        const lastKind = activityKind(last);
        if (lastKind && lastKind === nextKind && isGenericActivityLine(last) && !isGenericActivityLine(clean)) {
          entries[entries.length - 1] = clean;
        } else if (lastKind && lastKind === nextKind && !isGenericActivityLine(last) && isGenericActivityLine(clean)) {
          // Keep richer detail, drop regressive generic line.
        } else {
          entries.push(clean);
        }
      }
    }
  }

  const filtered = entries.filter((e) => !isTransientStatus(e));
  return {
    ...msg,
    activityEntries: filtered,
    activitySummary: summarizeActivityEntries(filtered, Boolean(msg.isGenerating)),
    toolCount: filtered.length,
  };
};

// ---------------------------------------------------------------------------
// Message role helpers
// ---------------------------------------------------------------------------

export const roleFromAgentId = (agentId: string): ChatMessage['role'] =>
  agentId === 'user' ? 'user' : 'agent';

export const normalizeMessageTextForDedup = (text: string) =>
  (text || '').replace(/\s+/g, ' ').trim();

// ---------------------------------------------------------------------------
// Generating message helpers
// ---------------------------------------------------------------------------

export const findLastGeneratingMessageIndex = (messages: ChatMessage[], agentId: string) => {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const msg = messages[i];
    if (msg.from === agentId && msg.isGenerating) return i;
  }
  return -1;
};

export const upsertGeneratingAgentMessage = (
  messages: ChatMessage[],
  agentId: string,
  text: string,
  activityLine?: string
): ChatMessage[] => {
  const idx = findLastGeneratingMessageIndex(messages, agentId);
  const now = new Date();
  if (idx >= 0) {
    const next = [...messages];
    let updated: ChatMessage = {
      ...next[idx],
      role: roleFromAgentId(agentId),
      from: agentId,
      to: next[idx].to || 'user',
      text,
      timestamp: now.toLocaleTimeString(),
      timestampMs: now.getTime(),
      isGenerating: true,
    };
    if (activityLine) {
      updated = addActivityEntry(updated, activityLine);
    }
    next[idx] = updated;
    return next;
  }
  let created: ChatMessage = {
    role: roleFromAgentId(agentId),
    from: agentId,
    to: 'user',
    text,
    timestamp: now.toLocaleTimeString(),
    timestampMs: now.getTime(),
    isGenerating: true,
  };
  if (activityLine) {
    created = addActivityEntry(created, activityLine);
  }
  return [
    ...messages,
    created,
  ];
};

export const appendGeneratingActivity = (messages: ChatMessage[], agentId: string, activityLine: string): ChatMessage[] => {
  const idx = findLastGeneratingMessageIndex(messages, agentId);
  const now = new Date();
  if (idx >= 0) {
    const next = [...messages];
    next[idx] = addActivityEntry(
      {
        ...next[idx],
        timestamp: now.toLocaleTimeString(),
        timestampMs: now.getTime(),
      },
      activityLine
    );
    return next;
  }
  return upsertGeneratingAgentMessage(messages, agentId, activityLine, activityLine);
};

// ---------------------------------------------------------------------------
// Subagent tree helpers
// ---------------------------------------------------------------------------

/** Update the subagentTree on a parent agent's generating message. */
export const updateParentSubagentTree = (
  messages: ChatMessage[],
  parentId: string,
  subagentId: string,
  updater: (entry: SubagentTreeEntry) => SubagentTreeEntry,
): ChatMessage[] => {
  let targetIdx = -1;
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if ((m.from || '').toLowerCase() === parentId.toLowerCase() && m.isGenerating) {
      targetIdx = i;
      break;
    }
  }
  if (targetIdx < 0) {
    for (let i = messages.length - 1; i >= 0; i--) {
      const m = messages[i];
      if ((m.from || '').toLowerCase() === parentId.toLowerCase() && m.role === 'agent') {
        targetIdx = i;
        break;
      }
    }
  }
  if (targetIdx < 0) return messages;

  const next = [...messages];
  const msg = next[targetIdx];
  const tree = msg.subagentTree ? [...msg.subagentTree] : [];
  const entryIdx = tree.findIndex((e) => e.subagentId === subagentId);
  if (entryIdx >= 0) {
    tree[entryIdx] = updater(tree[entryIdx]);
  }
  next[targetIdx] = { ...msg, subagentTree: tree };
  return next;
};

// ---------------------------------------------------------------------------
// Agent tree / workspace derivation
// ---------------------------------------------------------------------------

type ActivityEntry = {
  path: string;
  agent: string;
  status: string;
  lastModified: number;
};

const collectActivityEntries = (
  nodes: Record<string, AgentTreeItem> | undefined,
  out: ActivityEntry[]
) => {
  if (!nodes) return;
  Object.values(nodes).forEach((item) => {
    if (item.type === 'file') {
      if (!item.path || !item.agent) return;
      out.push({
        path: item.path,
        agent: item.agent,
        status: item.status || 'idle',
        lastModified: Number(item.last_modified || 0),
      });
      return;
    }
    collectActivityEntries(item.children, out);
  });
};

const splitFilePath = (path: string) => {
  const idx = path.lastIndexOf('/');
  if (idx < 0) {
    return { folder: '.', file: path };
  }
  return {
    folder: path.slice(0, idx) || '.',
    file: path.slice(idx + 1),
  };
};

export const buildAgentWorkInfo = (tree: Record<string, AgentTreeItem>): Record<string, AgentWorkInfo> => {
  const entries: ActivityEntry[] = [];
  collectActivityEntries(tree, entries);

  const byAgent = entries.reduce<Record<string, ActivityEntry[]>>((acc, entry) => {
    if (!acc[entry.agent]) acc[entry.agent] = [];
    acc[entry.agent].push(entry);
    return acc;
  }, {});

  const out: Record<string, AgentWorkInfo> = {};
  Object.entries(byAgent).forEach(([agent, list]) => {
    const active = list
      .filter((entry) => entry.status === 'working')
      .sort((a, b) => b.lastModified - a.lastModified);
    const current = active[0];
    if (!current) return;
    const parts = splitFilePath(current.path);
    out[agent] = {
      path: current.path,
      folder: parts.folder,
      file: parts.file,
      status: current.status,
      activeCount: active.length,
    };
  });

  return out;
};

export const normalizeAgentStatus = (
  status?: string
): 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working' => {
  if (status === 'calling_tool') return 'calling_tool';
  if (status === 'model_loading') return 'model_loading';
  if (status === 'thinking') return 'thinking';
  if (status === 'working') return 'working';
  return 'idle';
};

export const buildSubagentInfos = (
  tree: Record<string, AgentTreeItem>,
  mainAgentIds: string[],
  agentStatus: Record<string, 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working'>
): SubagentInfo[] => {
  const entries: ActivityEntry[] = [];
  collectActivityEntries(tree, entries);
  const mainSet = new Set(mainAgentIds.map((id) => id.toLowerCase()));

  const bySubagent = entries
    .filter((entry) => !mainSet.has(entry.agent.toLowerCase()))
    .reduce<Record<string, ActivityEntry[]>>((acc, entry) => {
      if (!acc[entry.agent]) acc[entry.agent] = [];
      acc[entry.agent].push(entry);
      return acc;
    }, {});

  const out: SubagentInfo[] = Object.entries(bySubagent)
    .reduce<SubagentInfo[]>((acc, [id, list]) => {
      const sorted = list.slice().sort((a, b) => b.lastModified - a.lastModified);
      const active = sorted.filter((entry) => entry.status === 'working');
      const current = active[0] || sorted[0];
      if (!current) return acc;

      const parts = splitFilePath(current.path);
      const uniquePaths = Array.from(new Set(sorted.map((entry) => entry.path))).slice(0, 8);
      const liveStatus = agentStatus[id];

      acc.push({
        id,
        status: liveStatus || (active.length > 0 ? 'working' : 'idle'),
        path: current.path,
        file: parts.file,
        folder: parts.folder,
        activeCount: active.length,
        paths: uniquePaths,
      });

      return acc;
    }, [])
    .sort((a, b) => {
      const score = (status: string) => (status === 'working' ? 2 : status === 'thinking' ? 1 : 0);
      return score(b.status) - score(a.status) || a.id.localeCompare(b.id);
    });

  return out;
};

// ---------------------------------------------------------------------------
// Message dedup & merge helpers
// ---------------------------------------------------------------------------

export const chatMessageKey = (msg: ChatMessage): string => {
  const from = msg.from || msg.role;
  const to = msg.to || '';
  const ts = msg.timestampMs ?? 0;
  return `${from}|${to}|${ts}|${msg.text}`;
};

export const sameMessageContent = (a: ChatMessage, b: ChatMessage): boolean => {
  const fromA = a.from || a.role;
  const fromB = b.from || b.role;
  const toA = a.to || '';
  const toB = b.to || '';
  return (
    fromA === fromB &&
    toA === toB &&
    normalizeMessageTextForDedup(a.text) === normalizeMessageTextForDedup(b.text)
  );
};

export const isStructuredAgentMessage = (msg: ChatMessage): boolean => {
  if ((msg.from || msg.role) === 'user') return false;
  try {
    const parsed = JSON.parse(msg.text);
    return typeof parsed?.type === 'string';
  } catch (_e) {
    return false;
  }
};

export const isPlanMessage = (msg: ChatMessage): boolean => {
  const text = (msg.text || '').trim();
  try {
    const parsed = JSON.parse(text);
    if (parsed?.type === 'plan' && !!parsed?.plan) return true;
  } catch { /* not pure JSON */ }
  if (text.includes('"type":"plan"') && text.includes('"plan":{')) return true;
  return false;
};

export const likelySameMessage = (a: ChatMessage, b: ChatMessage): boolean => {
  if (isPlanMessage(a) && isPlanMessage(b)) {
    return (a.from || a.role) === (b.from || b.role);
  }
  if (!sameMessageContent(a, b)) return false;
  if (isStructuredAgentMessage(a) || isStructuredAgentMessage(b)) return true;
  const ta = a.timestampMs ?? 0;
  const tb = b.timestampMs ?? 0;
  if (ta === 0 || tb === 0) return true;
  return Math.abs(ta - tb) <= 120_000;
};

/** Keep only the last plan message per agent; dedup non-plan by key. */
export const dedupPlanMessages = (messages: ChatMessage[]): ChatMessage[] => {
  const lastPlanIdx = new Map<string, number>();
  messages.forEach((m, idx) => {
    if (isPlanMessage(m)) {
      lastPlanIdx.set(m.from || m.role || '', idx);
    }
  });

  const seen = new Set<string>();
  return messages.filter((m, idx) => {
    if (isPlanMessage(m)) {
      const agent = m.from || m.role || '';
      return lastPlanIdx.get(agent) === idx;
    }
    const key = chatMessageKey(m);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
};

export const mergeChatMessages = (persisted: ChatMessage[], live: ChatMessage[]): ChatMessage[] => {
  if (persisted.length === 0) {
    // Keep generating messages and recent user messages (images are ephemeral
    // and would be lost if we dropped them before the server persists the text).
    return live.filter((m) => m.isGenerating || m.role === 'user' || m.from === 'user');
  }
  if (live.length === 0) return dedupPlanMessages(persisted);

  const persistedWithActivity = persisted.map((msg) => {
    const matchingLive = live.find(
      (candidate) =>
        !candidate.isGenerating &&
        likelySameMessage(msg, candidate) &&
        (
          (candidate.activityEntries && candidate.activityEntries.length > 0) ||
          (candidate.content && candidate.content.length > 0) ||
          candidate.subagentTree ||
          candidate.segments ||
          candidate.toolCount ||
          candidate.durationMs ||
          candidate.contextTokens ||
          (candidate.images && candidate.images.length > 0)
        )
    );
    if (!matchingLive) return msg;
    return {
      ...msg,
      content: matchingLive.content || msg.content,
      activityEntries: matchingLive.activityEntries || msg.activityEntries,
      activitySummary: matchingLive.activitySummary || msg.activitySummary,
      subagentTree: matchingLive.subagentTree || msg.subagentTree,
      segments: matchingLive.segments || msg.segments,
      toolCount: matchingLive.toolCount || msg.toolCount,
      durationMs: matchingLive.durationMs || msg.durationMs,
      contextTokens: matchingLive.contextTokens || msg.contextTokens,
      images: matchingLive.images || msg.images,
    };
  });

  const now = Date.now();
  const uniqueExtras = live.filter(
    (m) => {
      if (m.isGenerating) return true;
      if (persistedWithActivity.some((p) => likelySameMessage(p, m))) return false;
      if (m.role === 'user' || m.from === 'user') return true;
      const ts = m.timestampMs ?? now;
      return now - ts <= LIVE_MESSAGE_GRACE_MS;
    }
  );
  const merged = [...persistedWithActivity, ...uniqueExtras];

  return dedupPlanMessages(merged);
};

export const shouldHideInternalChatMessage = (_from?: string, text?: string): boolean => {
  if (!text) return false;
  if (text.startsWith('Starting autonomous loop for task:')) return true;
  // Hide raw tool observation messages that leak from context (e.g. "Tool Bash: Bash output...")
  if (/^Tool\s+\w+:/i.test(text)) return true;
  // Hide "Used tool: Read · target=..." lines rendered from sanitization
  if (text.startsWith('Used tool:')) return true;
  // Hide delegation task text (already shown in SubagentTreeView)
  if (text.startsWith('Delegated task:')) return true;
  return false;
};

/** Hide persisted-only tool-status messages that are redundant with content blocks.
 *  NOT used for live SSE messages (those drive spinner/activity). */
export const isPersistedToolOnlyMessage = (text: string): boolean =>
  /^(Reading file|Writing file|Editing file|Running command|Searching|Listing files|Fetching URL|Searching web|Delegating to subagent|Calling tool)[:.]/i.test(text);

// ---------------------------------------------------------------------------
// Reconstruct content blocks from persisted text
// ---------------------------------------------------------------------------

/** Tool start-phase patterns produced by sanitize_message_for_ui / tool_status_line(Start). */
const TOOL_START_PATTERNS: [RegExp, string][] = [
  [/^Reading file[:.]\s*(.*)/i, 'Read'],
  [/^Writing file[:.]\s*(.*)/i, 'Write'],
  [/^Editing file[:.]\s*(.*)/i, 'Edit'],
  [/^Running command[:.]\s*(.*)/i, 'Bash'],
  [/^Searching web[:.]\s*(.*)/i, 'WebSearch'],
  [/^Searching[:.]\s*(.*)/i, 'Grep'],
  [/^Listing files[:.]\s*(.*)/i, 'Glob'],
  [/^Fetching URL[:.]\s*(.*)/i, 'WebFetch'],
  [/^Delegating to subagent[:.]\s*(.*)/i, 'Task'],
  [/^Calling tool[:.]\s*(.*)/i, 'Tool'],
];

/** Result-phase lines to skip (redundant with start-phase lines). */
const isToolResultLine = (line: string): boolean =>
  /^Used tool:\s*\w+/i.test(line) ||
  /^Tool \w+:/i.test(line) ||
  /^Delegated task:/i.test(line);

/**
 * Parse persisted message text to reconstruct ContentBlock[] for tool activity.
 * After server restart, messages lose ephemeral tool blocks. This recovers
 * them from the "Reading file:", "Running command:", etc. status lines that
 * sanitize_message_for_ui embeds in the persisted text.
 */
export const reconstructContentFromText = (text: string): {
  content: ContentBlock[];
  toolCount: number;
} | null => {
  const lines = text.split('\n');
  const content: ContentBlock[] = [];
  let textBuffer: string[] = [];
  let toolCount = 0;
  let idCounter = 0;

  const flushText = () => {
    const t = textBuffer.join('\n').trim();
    if (t) content.push({ type: 'text', text: t });
    textBuffer = [];
  };

  for (const line of lines) {
    const trimmed = line.trim();

    // Match start-phase tool lines → create tool_use blocks
    let matched = false;
    for (const [re, toolName] of TOOL_START_PATTERNS) {
      const m = trimmed.match(re);
      if (m) {
        flushText();
        content.push({
          type: 'tool_use',
          id: `restored-${idCounter++}`,
          tool: toolName,
          args: m[1]?.trim() || '',
          status: 'done',
        });
        toolCount++;
        matched = true;
        break;
      }
    }
    if (matched) continue;

    // Skip redundant result lines
    if (isToolResultLine(trimmed)) continue;

    textBuffer.push(line);
  }

  flushText();

  if (toolCount === 0) return null;
  return { content, toolCount };
};
