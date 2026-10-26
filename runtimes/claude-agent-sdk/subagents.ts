type Send = (message: Record<string, unknown>) => void;

/** Preserve SDK ownership: this only translates identity and presentation events. */
export function subagentEvents(send: Send) {
  const tasks = new Map<string, { name: string; task: string }>();
  const tools = new Map<string, string>();
  const ensure = (id: string, name?: string, task = '', tool?: string) => {
    const previous = tasks.get(id);
    const value = { name: name || previous?.name || 'Claude subagent', task: previous?.task || task || '' };
    tasks.set(id, value);
    if (tool) tools.set(tool, id);
    if (!previous || previous.name !== value.name || previous.task !== value.task)
      send({ type:'subagent_start', agent_id:id, ...value });
    return id;
  };
  return {
    start: async (event: any) => { ensure(event.agent_id, event.agent_type); return {}; },
    stop: async (event: any) => {
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
      }
      return true;
    },
  };
}
