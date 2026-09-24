import type { ChatMessage } from '../../../types';
import { dedupeActivityEntries, isProgressLineText, summarizeCollapsedActivity } from './activity';

export const normalizeAgentKey = (value?: string) => (value || '').trim().toLowerCase();

export const normalizeMessageTextForDedup = (text?: string) =>
  (text || '').replace(/\s+/g, ' ').trim();

export const sortMessagesByTime = (messages: ChatMessage[]) =>
  messages
    .map((msg, index) => ({ msg, index }))
    .sort((a, b) => {
      const ta = a.msg.timestampMs ?? 0;
      const tb = b.msg.timestampMs ?? 0;
      if (ta <= 0 && tb <= 0) return a.index - b.index;
      if (ta <= 0) return 1;
      if (tb <= 0) return -1;
      if (ta !== tb) return ta - tb;
      return a.index - b.index;
    })
    .map((entry) => entry.msg);

export const roleFromSender = (sender: string): ChatMessage['role'] => {
  const key = normalizeAgentKey(sender);
  if (key === 'user') return 'user';
  return 'agent';
};

/** Strip embedded tool/plan/structured JSON from text using brace-depth counting. */
export const stripStructuredJsonFromText = (text: string): string => {
  const MARKERS = [
    '"type":"tool"', '"type":"plan"', '"type":"finalize_task"',
    '"type": "tool"', '"type": "plan"',
    '"name":"AskUser"', '"name": "AskUser"',
    '"name":"Read"', '"name": "Read"',
    '"name":"Write"', '"name": "Write"',
    '"name":"Edit"', '"name": "Edit"',
    '"name":"Bash"', '"name": "Bash"',
    '"name":"Glob"', '"name": "Glob"',
    '"name":"Grep"', '"name": "Grep"',
    '"name":"Skill"', '"name": "Skill"',
    '"name":"WebFetch"', '"name": "WebFetch"',
    '"name":"WebSearch"', '"name": "WebSearch"',
    '"name":"Task"', '"name": "Task"',
    '"name":"delegate_to_agent"', '"name": "delegate_to_agent"',
    '"name":"capture_screenshot"', '"name": "capture_screenshot"',
  ];
  // Strip <tool_call>...</tool_call> XML-style blocks (Qwen/Hermes format)
  let result = text.replace(/<\/?tool_call>/g, '');
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
      let depth = 0, end = -1;
      for (let i = start; i < result.length; i++) {
        if (result[i] === '{') depth++;
        else if (result[i] === '}') { depth--; if (depth === 0) { end = i + 1; break; } }
      }
      if (end < 0) break;
      result = result.slice(0, start) + result.slice(end);
    }
  }
  return result.trim();
};

export const START_AUTONOMOUS_LINE_RE = /^Starting autonomous loop for task:/i;
export const CONTENT_OMITTED_LINE_RE = /^\(content omitted in chat; open the file viewer for full text\)$/i;

export const parseToolNameFromLine = (line: string): string | null => {
  const trimmed = line.trim();
  if (!trimmed.startsWith('{')) return null;
  try {
    const parsed = JSON.parse(trimmed);
    if (parsed?.type === 'tool' && typeof parsed?.tool === 'string') return parsed.tool;
    if (
      typeof parsed?.type === 'string' &&
      parsed.type !== 'finalize_task' &&
      parsed.args &&
      typeof parsed.args === 'object'
    ) {
      return parsed.type;
    }
    if (typeof parsed?.name === 'string' && parsed.args && typeof parsed.args === 'object') {
      return parsed.name;
    }
  } catch (_e) {
    // ignore non-json
  }
  return null;
};

export const sanitizeAgentMessageText = (text: string) => {
  if (!text) return '';

  const withoutEmbedded = stripStructuredJsonFromText(text);
  const lines = withoutEmbedded.split('\n');
  const kept: string[] = [];
  let suppressedToolBody: 'read' | 'grep' | 'glob' | null = null;
  for (const line of lines) {
    const trimmed = line.trim();
    const lower = trimmed.toLowerCase();

    if (suppressedToolBody) {
      const isBoundary =
        trimmed.startsWith('{') ||
        lower.startsWith('tool ') ||
        lower.startsWith('tool_error:') ||
        lower.startsWith('tool_not_allowed:');
      const shouldSuppress =
        suppressedToolBody === 'read'
          ? !isBoundary
          : !isBoundary && trimmed.length > 0;
      if (shouldSuppress) continue;
      suppressedToolBody = null;
    }

    if (!trimmed) {
      if (kept.length > 0 && kept[kept.length - 1] !== '') kept.push('');
      continue;
    }
    if (parseToolNameFromLine(trimmed)) continue;

    if (lower.startsWith('tool read:')) {
      const match = /tool\s+read:\s*read:\s*(.+?)(?:\s+\(truncated:|$)/i.exec(trimmed);
      const target = match?.[1]?.trim();
      kept.push(target ? `Used tool: Read · target=${target}` : 'Used tool: Read');
      suppressedToolBody = 'read';
      continue;
    }
    if (lower.startsWith('tool grep:')) {
      kept.push('Used tool: Grep');
      suppressedToolBody = 'grep';
      continue;
    }
    if (lower.startsWith('tool glob:')) {
      kept.push('Used tool: Glob');
      suppressedToolBody = 'glob';
      continue;
    }
    if (lower.startsWith('tool websearch:')) {
      const query = trimmed
        .split('WebSearch:')
        .pop()
        ?.split('(')[0]
        ?.trim()
        .replace(/^"|"$/g, '');
      kept.push(query ? `Used tool: WebSearch · target=${query}` : 'Used tool: WebSearch');
      continue;
    }

    if (START_AUTONOMOUS_LINE_RE.test(trimmed)) continue;
    if (CONTENT_OMITTED_LINE_RE.test(trimmed)) continue;
    kept.push(line);
  }
  return kept.join('\n').replace(/\n{3,}/g, '\n\n').trim();
};

// Derived rows are cached per source message (and per synthesized key) so
// the same input row yields the same output object on every pass; without
// it every agent row was a new object per streamed token and no row memo
// could hold.
const derivedCache = new WeakMap<ChatMessage, { sig: string; out: ChatMessage }>();
const derive = (msg: ChatMessage, sig: string, build: () => ChatMessage): ChatMessage => {
  const hit = derivedCache.get(msg);
  if (hit && hit.sig === sig) return hit.out;
  const out = build();
  derivedCache.set(msg, { sig, out });
  return out;
};
const synthCache = new Map<string, ChatMessage>();
const synth = (sig: string, build: () => ChatMessage): ChatMessage => {
  const hit = synthCache.get(sig);
  if (hit) return hit;
  if (synthCache.size > 500) synthCache.clear();
  const out = build();
  synthCache.set(sig, out);
  return out;
};

export const collapseProgressMessages = (messages: ChatMessage[]): ChatMessage[] => {
  const out: ChatMessage[] = [];
  const pendingByAgent = new Map<string, string[]>();
  const pendingTsByAgent = new Map<string, number>();
  const pendingGeneratingByAgent = new Map<string, boolean>();

  const appendPendingToOutput = (agentId: string, to?: string) => {
    const pending = pendingByAgent.get(agentId);
    if (!pending || pending.length === 0) return;
    const ts = pendingTsByAgent.get(agentId);
    const isGenerating = !!pendingGeneratingByAgent.get(agentId);
    const deduped = dedupeActivityEntries(pending);
    if (deduped.length === 0) {
      pendingByAgent.delete(agentId);
      pendingTsByAgent.delete(agentId);
      pendingGeneratingByAgent.delete(agentId);
      return;
    }
    const sig = [agentId, to || 'user', ts ?? '', isGenerating ? 1 : 0, ...deduped].join('\u0001');
    out.push(synth(sig, () => ({
      role: roleFromSender(agentId),
      from: agentId,
      to: to || 'user',
      text: '',
      timestamp: ts ? new Date(ts).toLocaleTimeString() : '',
      timestampMs: ts,
      isGenerating,
      activityEntries: deduped,
      activitySummary: summarizeCollapsedActivity(deduped, isGenerating),
    })));
    pendingByAgent.delete(agentId);
    pendingTsByAgent.delete(agentId);
    pendingGeneratingByAgent.delete(agentId);
  };

  const activityEntriesForMsg = (msg: ChatMessage): string[] => {
    const entries = Array.isArray(msg.activityEntries) ? msg.activityEntries : [];
    if (entries.length > 0) return dedupeActivityEntries(entries);
    if (isProgressLineText(msg.text)) return dedupeActivityEntries([msg.text]);
    return [];
  };

  for (const msg of messages) {
    if (msg.role === 'user') {
      for (const key of Array.from(pendingByAgent.keys())) {
        appendPendingToOutput(key, msg.to);
      }
      out.push(msg);
      continue;
    }

    const agentId = normalizeAgentKey(msg.from || msg.role);
    const entries = activityEntriesForMsg(msg);
    const body = String(msg.text || '').trim();
    const hasContentBlocks = msg.content && msg.content.length > 0;
    const onlyProgress = !hasContentBlocks && (!body || isProgressLineText(body));

    if (onlyProgress) {
      if (entries.length > 0) {
        const existing = pendingByAgent.get(agentId) || [];
        pendingByAgent.set(agentId, dedupeActivityEntries([...existing, ...entries]));
        if (!pendingTsByAgent.has(agentId) && msg.timestampMs) {
          pendingTsByAgent.set(agentId, msg.timestampMs);
        }
        if (msg.isGenerating) {
          pendingGeneratingByAgent.set(agentId, true);
        }
      }
      continue;
    }

    if (pendingByAgent.has(agentId)) {
      const isGenerating = !!pendingGeneratingByAgent.get(agentId) || !!msg.isGenerating;
      const merged = dedupeActivityEntries([
        ...(pendingByAgent.get(agentId) || []),
        ...entries,
      ]);
      pendingByAgent.delete(agentId);
      pendingTsByAgent.delete(agentId);
      pendingGeneratingByAgent.delete(agentId);
      const sig = ['merged', isGenerating ? 1 : 0, ...merged].join('\u0001');
      out.push(derive(msg, sig, () => ({
        ...msg,
        isGenerating,
        activityEntries: merged.length > 0 ? merged : msg.activityEntries,
        activitySummary:
          merged.length > 0
            ? summarizeCollapsedActivity(merged, isGenerating)
            : msg.activitySummary,
      })));
      continue;
    }

    if (entries.length === 0) {
      out.push(msg);
      continue;
    }
    out.push(derive(msg, ['solo', ...entries].join('\u0001'), () => ({
      ...msg,
      activityEntries: entries,
      activitySummary: summarizeCollapsedActivity(entries, !!msg.isGenerating),
    })));
  }

  for (const key of Array.from(pendingByAgent.keys())) {
    appendPendingToOutput(key);
  }

  return out;
};
