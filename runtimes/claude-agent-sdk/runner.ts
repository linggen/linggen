// JSON-lines bridge. Only stdout carries protocol; credentials stay in env.
import { createInterface } from 'node:readline';
import { subagentEvents, taskEvent } from './subagents';

type Send = (message: Record<string, unknown>) => void;
type Rpc = (kind: string, input: any, signal?: AbortSignal) => Promise<any>;

// Rehydrate only the SDK's user-approved session rules through its own settings
// interface. No rule is evaluated by Linggen and no project/global file changes.
export function sessionPermissionSettings(updates: any[] = []) {
  const rules: Record<string, Set<string>> = { allow:new Set(), deny:new Set(), ask:new Set() };
  const directories = new Set<string>();
  for (const update of updates) {
    const target = rules[update.behavior];
    if (target && ['addRules','replaceRules','removeRules'].includes(update.type)) {
      if (update.type === 'replaceRules') target.clear();
      for (const rule of update.rules || []) {
        const value = rule.ruleContent ? `${rule.toolName}(${rule.ruleContent})` : rule.toolName;
        if (update.type === 'removeRules') target.delete(value); else target.add(value);
      }
    }
    if (update.type === 'addDirectories') for (const path of update.directories || []) directories.add(path);
    if (update.type === 'removeDirectories') for (const path of update.directories || []) directories.delete(path);
  }
  return { permissions: { allow:[...rules.allow], deny:[...rules.deny], ask:[...rules.ask], additionalDirectories:[...directories] } };
}

// Observe calls without making permission decisions. The SDK's own rules and
// modes run before canUseTool asks the host to render a widget.
export function observeTool(send: Send) {
  return async (input: any) => {
    send({ type: 'tool_start', tool: input.tool_name, args: input.tool_input, id: input.tool_use_id,
      ...(input.agent_id ? { agent_id: input.agent_id } : {}) });
    return {};
  };
}

// The main thread's shell moved: the host moves the session's folder with it.
// The SDK runs CwdChanged only for settings-file hooks, never for a host's
// callback hooks, so the folder is read off each tool hook's own `cwd`. The
// first main-thread reading is the baseline (the SDK reports it resolved, e.g.
// /private/tmp for /tmp); a later different reading is a cd. A subagent's
// hooks (their input carries agent_id) are its own business.
export function followCwd(send: Send) {
  let known: string | undefined;
  return (input: any) => {
    if (input.agent_id || !input.cwd || input.cwd === known) return;
    if (known !== undefined) send({ type: 'cwd', cwd: input.cwd });
    known = input.cwd;
  };
}

const EDIT_TOOLS = new Set(['Edit', 'Write', 'MultiEdit', 'NotebookEdit']);

// Whether the SDK decides this tool by itself under `mode`, so an open prompt
// for it is moot. Moot prompts are withdrawn for the SDK to retry; the rest
// stay open for the user, as Claude Code re-checks its queue on a mode change.
export function modeSettles(mode: string, tool: string) {
  if (mode === 'acceptEdits') return EDIT_TOOLS.has(tool);
  return ['auto', 'bypassPermissions', 'dontAsk'].includes(mode);
}

export const MODE_CHANGED = 'Permission mode changed. Retry this operation under the new mode.';

// An ask for a call that began before the latest mode change was decided
// under the old mode; it can reach the host after the SDK acknowledged the
// change. When the new mode settles it, it is withdrawn like an open prompt.
export function staleAsk(startedEpoch: number | undefined, epoch: number, mode: string, tool: string) {
  return startedEpoch !== undefined && startedEpoch < epoch && modeSettles(mode, tool);
}

// A folder's own settings load only once the owner trusted it.
export function settingSources(input: any) {
  return Array.isArray(input.settingSources) ? input.settingSources : ['user'];
}

// The host's per-turn memory (recall + capture nudge) reaches the model as
// the SDK's own prompt context, never as text in the user's message. Each
// message the host sends carries its own; the hook takes them in order.
export function turnContext(memories: string[]) {
  return async () => {
    const text = memories.shift();
    return text ? { hookSpecificOutput: { hookEventName: 'UserPromptSubmit' as const, additionalContext: text } } : {};
  };
}

export async function permissionRequest(tool: string, input: any, rpc: Rpc, options: any = {}) {
  if (tool === 'AskUserQuestion') {
    const response = await rpc('ask_user', { ...input, ...(options.agentID ? { agent_id: options.agentID } : {}) }, options.signal);
    if (!response.answers) return { behavior: 'deny' as const, message: 'Question cancelled.' };
    return { behavior: 'allow' as const, updatedInput: { ...input, answers: response.answers } };
  }
  const response = await rpc('permission', { tool, args: input,
    title: options.title, description: options.description, reason: options.decisionReason,
    suggestions: options.suggestions || [], ...(options.agentID ? { agent_id: options.agentID } : {}) }, options.signal);
  if (!response.allow) return { behavior: 'deny' as const, message: response.reason || 'User declined this tool call.' };
  if (response.mode) {
    try { await options.changeMode(response.mode); }
    catch (error) { return { behavior: 'deny' as const, message: String(error) }; }
  }
  const updates = response.remember ? (options.suggestions || []).filter((suggestion: any) => suggestion.type !== 'setMode')
    .map((suggestion: any) => ({ ...suggestion, destination:'session' })) : [];
  if (updates.length) {
    const saved = await rpc('permission_updates', { updates });
    if (saved.error) return { behavior:'deny' as const, message:saved.error };
  }
  return { behavior: 'allow' as const, updatedInput: input,
    ...(response.remember ? { updatedPermissions:updates } : {}) };
}

// What the person typed reaches the SDK as theirs: `origin: human` is what
// lets a `now` message move running work aside instead of cutting it off.
export function userMessage(input: { prompt: string; images?: string[] }, priority?: 'now', uuid?: string) {
  const content: any[] = [{ type: 'text', text: input.prompt }];
  for (const image of input.images || []) {
    const match = /^data:(image\/[\w.+-]+);base64,(.+)$/s.exec(image);
    if (!match) throw new Error('Expected a base64 image data URL');
    content.push({ type: 'image', source: { type: 'base64', media_type: match[1], data: match[2] } });
  }
  return { type: 'user' as const, session_id: '', parent_tool_use_id: null,
    message: { role: 'user' as const, content }, origin: { kind: 'human' as const },
    ...(priority ? { priority } : {}), ...(uuid ? { uuid } : {}) };
}

/** Messages a running turn takes in after its first: each one the person
 *  chose to send now from the queue. Closed once the turn's work is done. */
export function inbox<T>() {
  const items: T[] = [];
  let wake: (() => void) | undefined;
  let closed = false;
  return {
    get closed() { return closed; },
    push(item: T) { items.push(item); wake?.(); },
    close() { closed = true; wake?.(); },
    async *drain() {
      while (true) {
        while (items.length) yield items.shift()!;
        if (closed) return;
        await new Promise<void>((resolve) => { wake = resolve; });
        wake = undefined;
      }
    },
  };
}

export function queryPrompt(first: any, later?: ReturnType<typeof inbox<any>>) {
  return (async function* () {
    yield first;
    if (later) yield* later.drain();
  })();
}

async function main() {
  const { query, createSdkMcpServer, tool } = await import('@anthropic-ai/claude-agent-sdk');
  const { z } = await import('zod');
  const send: Send = message => process.stdout.write(`${JSON.stringify(message)}\n`);
  const subagents = subagentEvents(send);
  const controller = new AbortController();
  const pending = new Map<number, { resolve: (response: any) => void; kind: string; tool?: string }>();
  let conversation: any;
  let markReady: () => void;
  const ready = new Promise<void>((resolve) => { markReady = resolve; });
  // The session's messages after the first, for as long as the host keeps
  // this runner (doc/background-tasks-spec.md): a turn's result does not end
  // it; the host's `end` does, once the session is idle.
  const later = inbox<any>();
  const finishInput = () => later.close();
  const memories: string[] = [];
  let mode = 'default';
  let modeEpoch = 0;
  const started = new Map<string, number>();
  let modeQueue = Promise.resolve();
  const cancelPermissions = () => {
    for (const [id, item] of pending) {
      if (item.kind !== 'permission' || !modeSettles(mode, item.tool || '')) continue;
      item.resolve({ allow: false, reason: MODE_CHANGED });
      pending.delete(id);
      send({ type: 'cancel_request', id });
    }
  };
  const changeMode = async (value: string) => {
    await ready;
    await conversation.setPermissionMode(value);
    mode = value;
    modeEpoch++;
    cancelPermissions();
    send({ type: 'status', permissionMode: mode });
  };
  let seq = 0;
  let start: (input: any) => void;
  const config = new Promise<any>(resolve => { start = resolve; });
  const lines = createInterface({ input: process.stdin });
  lines.on('line', line => {
    const message = JSON.parse(line);
    // The first message's memory, before any later one's.
    if (message.type === 'start') { memories.push(message.turnMemory || ''); start(message); }
    if (message.type === 'reply') {
      pending.get(message.id)?.resolve(message);
      pending.delete(message.id);
    }
    if (message.type === 'mode') {
      modeQueue = modeQueue.then(async () => {
        try {
          await changeMode(message.mode);
          send({ type: 'mode_result', id: message.id, ok: true });
        } catch (error) {
          send({ type: 'mode_result', id: message.id, ok: false, error: String(error) });
        }
      });
    }
    // A new turn, or (`steer`) a queued message the person sends into the
    // running one now. The host decides which; this only delivers.
    if (message.type === 'prompt' || message.type === 'steer') {
      try {
        if (later.closed) throw new Error('This runner has ended.');
        const steer = message.type === 'steer';
        later.push(userMessage({ prompt: message.text, images: message.images }, steer ? 'now' : undefined, message.uuid));
        memories.push(message.turnMemory || '');
        send({ type: 'mode_result', id: message.id, ok: true });
      } catch (error) {
        send({ type: 'mode_result', id: message.id, ok: false, error: String(error) });
      }
    }
    // Stop on the turn: the turn ends, background tasks keep running.
    if (message.type === 'interrupt') {
      void (async () => {
        try { await ready; await conversation.interrupt(); send({ type:'mode_result', id:message.id, ok:true }); }
        catch (error) { send({ type:'mode_result', id:message.id, ok:false, error:String(error) }); }
      })();
    }
    if (message.type === 'end') finishInput();
    if (message.type === 'stop_task') {
      void (async () => {
        try { await ready; await conversation.stopTask(message.task_id); send({ type:'mode_result', id:message.id, ok:true }); }
        catch (error) { send({ type:'mode_result', id:message.id, ok:false, error:String(error) }); }
      })();
    }
    if (message.type === 'cancel') controller.abort();
  });
  lines.on('close', () => controller.abort());
  const rpc: Rpc = (kind, input, signal) => new Promise(resolve => {
    const id = ++seq;
    const cancel = () => {
      pending.delete(id);
      resolve({ allow:false });
      send({ type:'cancel_request', id });
    };
    if (signal?.aborted) { resolve({ allow:false }); return; }
    signal?.addEventListener('abort', cancel, { once:true });
    pending.set(id, { resolve: (response) => { signal?.removeEventListener('abort', cancel); resolve(response); }, kind, tool: input?.tool });
    send({ type: 'request', kind, id, input });
  });
  const input = await config;
  const env = { ...process.env };
  for (const key of ['ANTHROPIC_AUTH_TOKEN', 'CLAUDE_CODE_OAUTH_TOKEN', 'CLAUDECODE',
    'CLAUDE_CODE_USE_BEDROCK', 'CLAUDE_CODE_USE_VERTEX', 'CLAUDE_CODE_USE_FOUNDRY']) delete env[key];
  env.CLAUDE_AGENT_SDK_CLIENT_APP = 'linggen';
  const cwd = followCwd(send);
  const endTool = async (event: any) => {
    cwd(event);
    started.delete(event.tool_use_id);
    let result = event.tool_response ?? event.error ?? '';
    let additionalContext: string | undefined;
    if (result?.type === 'image' && result.file?.base64) {
      const shown = await rpc('image', { base64: result.file.base64, caption: event.tool_input?.file_path || 'Image',
        ...(event.agent_id ? { agent_id: event.agent_id } : {}) });
      result = { type:'image', reference:shown.reference, error:shown.error };
      if (shown.reference) {
        additionalContext = `Linggen kept this image for display. Include [${shown.reference}] in your answer to show it.`;
      }
    }
    send({ type: 'tool_end', id: event.tool_use_id, tool: event.tool_name,
      ...(event.agent_id ? { agent_id: event.agent_id } : {}),
      failed: event.hook_event_name === 'PostToolUseFailure',
      result });
    return additionalContext ? { hookSpecificOutput: { hookEventName: 'PostToolUse', additionalContext } } : {};
  };
  try {
    const hostTools = createSdkMcpServer({ name: 'linggen-display', version: '1.0.0', tools: [
      tool('DisplayImage', 'Display a local or online image in this Linggen chat, with a thumbnail and click to enlarge. Returns an image reference to include in your answer.',
        { source: z.string().describe('Local image path or https image URL'), caption: z.string().optional() },
        async (args) => {
          const shown = await rpc('display_image', args);
          return { content: [{ type: 'text' as const, text: shown.reference ? `Include [${shown.reference}] in your answer to show this image.` : shown.error || 'Image could not be displayed.' }], isError: !shown.reference };
        }),
    ] });
    const sessionSettings = sessionPermissionSettings(input.permissionUpdates);
    conversation = query({
      prompt: input.validateOnly
        ? queryPrompt({ type: 'user', session_id: '', parent_tool_use_id: null, message: { role: 'user', content: '/context' } })
        : queryPrompt(userMessage(input, undefined, input.uuid), later),
      options: {
        cwd: input.cwd, model: input.model, env, settingSources: settingSources(input),
        resume: input.resume || undefined, persistSession: !input.validateOnly,
        systemPrompt: { type: 'preset', preset: 'claude_code', ...(input.sessionMemory ? { append: input.sessionMemory } : {}) },
        tools: { type: 'preset', preset: 'claude_code' },
        disallowedTools: input.disallowedTools || [],
        settings: sessionSettings,
        allowedTools: sessionSettings.permissions.allow,
        additionalDirectories: sessionSettings.permissions.additionalDirectories,
        includePartialMessages: true, forwardSubagentText: true, promptSuggestions: true, maxTurns: input.maxTurns,
        maxBudgetUsd: input.maxBudgetUsd,
        permissionMode: input.validateOnly ? 'default' : input.permissionMode || 'default',
        abortController: controller,
        mcpServers: input.validateOnly ? {} : { ...(input.mcpServers || {}), 'linggen-display': hostTools },
        hooks: {
          SubagentStart: [{ hooks: [subagents.start] }],
          SubagentStop: [{ hooks: [subagents.stop] }],
          UserPromptSubmit: [{ hooks: [turnContext(memories)] }],
          PreToolUse: [{ hooks: [observeTool(send), async (event: any) => {
            started.set(event.tool_use_id, modeEpoch);
            cwd(event);
            return {};
          }] }],
          PostToolUse: [{ hooks: [endTool] }],
          PostToolUseFailure: [{ hooks: [endTool] }],
        },
        canUseTool: async (tool, args, options) => {
          const startedEpoch = started.get(options.toolUseID);
          started.delete(options.toolUseID);
          const result = staleAsk(startedEpoch, modeEpoch, mode, tool)
            ? { behavior: 'deny' as const, message: MODE_CHANGED }
            : await permissionRequest(tool, args, rpc, { ...options, changeMode });
          if (result.behavior === 'deny') send({ type: 'tool_end', id: options.toolUseID,
            ...(options.agentID ? { agent_id: options.agentID } : {}),
            tool, failed: true, result: result.message });
          return result;
        },
      },
    });
    markReady!();
    if (input.validateOnly) {
      await conversation.initializationResult();
      try { await conversation.setPermissionMode(input.permissionMode); send({ type: 'mode_result', ok: true, commands: await conversation.supportedCommands() }); }
      catch (error) { send({ type: 'mode_result', ok: false, error: String(error) }); }
      controller.abort();
      return;
    }
    for await (const message of conversation) {
      const m = message as any;
      const task = taskEvent(m);
      if (task) send(task);
      if (subagents.route(m)) continue;
      if (m.type === 'system' && m.subtype === 'init') {
        mode = m.permissionMode;
        send({ type: 'init', session_id: m.session_id, model: m.model,
          permissionMode: mode, commands: await conversation.supportedCommands() });
      } else if (m.type === 'system' && m.subtype === 'status') {
        send({ type: 'status', status: m.status, permissionMode: m.permissionMode });
      } else if (m.type === 'system' && m.subtype === 'commands_changed') {
        send({ type: 'commands', commands: m.commands });
      } else if (m.type === 'system' && m.subtype === 'local_command_output') {
        send({ type: 'text', text: m.content });
      } else if (m.type === 'system' && m.subtype === 'compact_boundary') {
        send({ type: 'text', text: 'Conversation compacted.' });
      } else if (m.type === 'conversation_reset') {
        send({ type: 'reset', session_id: m.new_conversation_id });
      } else if (m.type === 'stream_event' && m.event?.delta?.type === 'text_delta') {
        send({ type: 'token', text: m.event.delta.text });
      } else if (m.type === 'assistant') {
        const text = (m.message?.content || []).filter((b: any) => b.type === 'text')
          .map((b: any) => b.text).join('\n');
        if (text) send({ type: 'text', text });
      } else if (m.type === 'result') {
        // Which of the host's messages this turn answered: it completes their runs.
        const uuids = m.user_message_uuids || (m.user_message_uuid ? [m.user_message_uuid] : []);
        send({ type: 'result', success: m.subtype === 'success' && !m.is_error,
          text: m.result, errors: m.errors, cost: m.total_cost_usd,
          session_id: m.session_id, usage: m.usage, uuids });
        // The window as it stands now, from the last request's usage: a
        // turn's summed usage counts the cache once per call and overshoots.
        conversation.getContextUsage({ detail: 'summary' })
          .then((c: any) => send({ type: 'context', tokens: c.totalTokens, limit: c.maxTokens }))
          .catch(() => {});
      } else if (m.type === 'prompt_suggestion') {
        if (m.suggestion) send({ type: 'suggestion', text: m.suggestion });
      } else if (m.type === 'active_goal') {
        send({ type: 'goal', active: m.value != null });
      }
      // A task's end is never main-thread text: the session task list shows
      // it (`taskEvent`), and the SDK tells the agent that launched it in a
      // turn of its own, whose output is what starts a run here.
    }
    send({ type: 'done' });
  } catch (error) {
    send({ type: 'error', message: error instanceof Error ? error.message : String(error) });
    process.exitCode = 1;
  } finally {
    finishInput();
    lines.close();
    for (const item of pending.values()) item.resolve({ allow: false });
  }
}

if (import.meta.main) await main();
