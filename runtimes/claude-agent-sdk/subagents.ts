type Send = (message: Record<string, unknown>) => void;

const TASK_KINDS: Record<string, string> = {
  local_bash: 'shell', local_agent: 'subagent', monitor: 'watch', mcp_task: 'watch', local_workflow: 'workflow',
};

/** The SDK's stand-in summary for a child that handed back its report as a
 *  message: it says nothing of the result, so it is no summary. */
const HANDED_BACK = 'report was delivered to you as a message';
const placeholder = (text?: string) => !!text && text.includes(HANDED_BACK);

/** A child's final report: the input of its SubagentHandback call. */
function handback(content: any[] = []): string | undefined {
  const call = content.find((block: any) => block.type === 'tool_use' && block.name === 'SubagentHandback');
  return typeof call?.input?.message === 'string' ? call.input.message : undefined;
}

/** The window a child's latest request filled: its whole prompt, cached
 *  or not. The SDK reports the three parts apart. */
export function contextTokens(usage: any): number {
  if (!usage) return 0;
  return (usage.input_tokens || 0) + (usage.cache_read_input_tokens || 0) + (usage.cache_creation_input_tokens || 0);
}

export function taskKind(type?: string) {
  return TASK_KINDS[type || ''] || type || 'task';
}

/** The SDK's background task frames, for the session task list: the live
 *  set whenever it changes, and each task's start, progress and end. */
export function taskEvent(m: any): Record<string, unknown> | null {
  if (m?.type !== 'system') return null;
  if (m.subtype === 'background_tasks_changed')
    return { type:'tasks', tasks:(m.tasks || []).map((t: any) => ({ id:t.task_id, kind:taskKind(t.task_type), title:t.description || '' })) };
  if (m.subtype === 'task_started')
    return { type:'task', id:m.task_id, kind:taskKind(m.task_type), title:m.description || '', background:m.is_backgrounded === true };
  if (m.subtype === 'task_progress')
    return { type:'task', id:m.task_id, activity:m.summary || m.last_tool_name || m.description || '' };
  if (m.subtype === 'task_notification')
    return { type:'task', id:m.task_id, status:m.status, summary:placeholder(m.summary) ? '' : m.summary || '', output_file:m.output_file || '' };
  return null;
}

/** Preserve SDK ownership: this only translates identity and presentation events. */
export function subagentEvents(send: Send) {
  const tasks = new Map<string, { name: string; task: string }>();
  const tools = new Map<string, string>();
  // Shells, watches and subagents a child launched: their ends are told to
  // that child by the SDK, never to the main thread.
  const launched = new Set<string>();
  const windows = new Map<string, number>();
  const ensure = (id: string, name?: string, task = '', tool?: string) => {
    const previous = tasks.get(id);
    const value = { name: name || previous?.name || 'Claude subagent', task: previous?.task || task || '' };
    tasks.set(id, value);
    if (tool) tools.set(tool, id);
    if (!previous || previous.name !== value.name || previous.task !== value.task)
      send({ type:'subagent_start', agent_id:id, ...value });
    return id;
  };
  // The SDK's own helpers (the /compact summarizer) fire the Subagent hooks
  // too, with no agent type and no task: no one asked for them, so they get
  // no row. A real child has a type, or a task_started that registered it.
  const internal = (event: any) => !event.agent_type && !tasks.has(event.agent_id);
  return {
    start: async (event: any) => { if (!internal(event)) ensure(event.agent_id, event.agent_type); return {}; },
    stop: async (event: any) => {
      if (internal(event)) return {};
      ensure(event.agent_id, event.agent_type);
      send({ type:'subagent_end', agent_id:event.agent_id, text:event.last_assistant_message, status:'completed' });
      return {};
    },
    /** Returns true when the frame belongs to a child and must not reach main chat. */
    route(message: any): boolean {
      const m = message;
      if (m.type === 'system' && m.subtype === 'task_started' && (m.task_type === 'local_agent' || m.subagent_type)) {
        ensure(m.task_id, m.subagent_type, m.prompt || m.description, m.tool_use_id);
        return true;
      }
      if (m.type === 'system' && m.subtype === 'task_started' && tasks.has(m.parent_task_id)) {
        launched.add(m.task_id);
        return true;
      }
      if (m.type === 'system' && launched.has(m.task_id)) {
        if (m.subtype === 'task_notification') launched.delete(m.task_id);
        return true;
      }
      if (m.type === 'system' && m.subtype === 'task_progress' && tasks.has(m.task_id)) {
        send({ type:'subagent_progress', agent_id:m.task_id, usage:m.usage, activity:m.summary || m.last_tool_name });
        return true;
      }
      if (m.type === 'system' && m.subtype === 'task_notification' && tasks.has(m.task_id)) {
        send({ type:'subagent_end', agent_id:m.task_id, text:m.summary, status:m.status, usage:m.usage });
        return true;
      }
      if (m.type !== 'assistant' && m.type !== 'stream_event' && m.type !== 'user') return false;
      if (!m.agent_id && !m.parent_tool_use_id) return false;
      const id = m.agent_id || tools.get(m.parent_tool_use_id) || `tool-${m.parent_tool_use_id}`;
      ensure(id, m.subagent_type || tasks.get(id)?.name, m.task_description, m.parent_tool_use_id);
      if (m.type === 'assistant') {
        const text = (m.message?.content || []).filter((block: any) => block.type === 'text').map((block: any) => block.text).join('\n');
        if (text) send({ type:'subagent_text', agent_id:id, text });
        const report = handback(m.message?.content);
        if (report) send({ type:'subagent_report', agent_id:id, text:report });
        const tokens = contextTokens(m.message?.usage);
        if (tokens && windows.get(id) !== tokens) {
          windows.set(id, tokens);
          send({ type:'subagent_context', agent_id:id, tokens });
        }
      }
      return true;
    },
  };
}
