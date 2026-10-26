// JSON-lines bridge. Only stdout carries protocol; credentials stay in env.
import { createInterface } from 'node:readline';
import { subagentEvents } from './subagents';

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

const EDIT_TOOLS = new Set(['Edit', 'Write', 'MultiEdit', 'NotebookEdit']);

// Whether the SDK decides this tool by itself under `mode`, so an open prompt
// for it is moot. Moot prompts are withdrawn for the SDK to retry; the rest
// stay open for the user, as Claude Code re-checks its queue on a mode change.
export function modeSettles(mode: string, tool: string) {
  if (mode === 'acceptEdits') return EDIT_TOOLS.has(tool);
  return ['auto', 'bypassPermissions', 'dontAsk'].includes(mode);
}

// A folder's own settings load only once the owner trusted it.
export function settingSources(input: any) {
  return Array.isArray(input.settingSources) ? input.settingSources : ['user'];
}

// The host's per-turn memory (recall + capture nudge) reaches the model as
// the SDK's own prompt context, never as text in the user's message.
export function turnContext(text?: string) {
  return async () => text ? { hookSpecificOutput: { hookEventName: 'UserPromptSubmit' as const, additionalContext: text } } : {};
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

export function queryPrompt(input: any, hold?: Promise<void>) {
  const content: any[] = [{ type: 'text', text: input.prompt }];
  for (const image of input.images || []) {
    const match = /^data:(image\/[\w.+-]+);base64,(.+)$/s.exec(image);
    if (!match) throw new Error('Expected a base64 image data URL');
    content.push({ type: 'image', source: { type: 'base64', media_type: match[1], data: match[2] } });
  }
  return (async function* () {
    yield { type: 'user' as const, session_id: '', parent_tool_use_id: null,
      message: { role: 'user' as const, content } };
    if (hold) await hold;
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
  let finishInput: () => void;
  const inputFinished = new Promise<void>((resolve) => { finishInput = resolve; });
  let mode = 'default';
  let modeQueue = Promise.resolve();
  const cancelPermissions = () => {
    for (const [id, item] of pending) {
      if (item.kind !== 'permission' || !modeSettles(mode, item.tool || '')) continue;
      item.resolve({ allow: false, reason: 'Permission mode changed. Retry this operation under the new mode.' });
      pending.delete(id);
      send({ type: 'cancel_request', id });
    }
  };
  const changeMode = async (value: string) => {
    await ready;
    await conversation.setPermissionMode(value);
    mode = value;
    cancelPermissions();
    send({ type: 'status', permissionMode: mode });
  };
  let seq = 0;
  let start: (input: any) => void;
  const config = new Promise<any>(resolve => { start = resolve; });
  const lines = createInterface({ input: process.stdin });
  lines.on('line', line => {
    const message = JSON.parse(line);
    if (message.type === 'start') start(message);
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
  const endTool = async (event: any) => {
    let result = event.tool_response ?? event.error ?? '';
    let additionalContext: string | undefined;
    if (result?.type === 'image' && result.file?.base64) {
      const shown = await rpc('image', { base64: result.file.base64, caption: event.tool_input?.file_path || 'Image' });
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
      prompt: queryPrompt(input.validateOnly ? { prompt: '/context' } : input, inputFinished),
      options: {
        cwd: input.cwd, model: input.model, env, settingSources: settingSources(input),
        resume: input.resume || undefined, persistSession: !input.validateOnly,
        systemPrompt: { type: 'preset', preset: 'claude_code', ...(input.sessionMemory ? { append: input.sessionMemory } : {}) },
        tools: { type: 'preset', preset: 'claude_code' },
        disallowedTools: input.disallowedTools || [],
        settings: sessionSettings,
        allowedTools: sessionSettings.permissions.allow,
        additionalDirectories: sessionSettings.permissions.additionalDirectories,
        includePartialMessages: true, forwardSubagentText: true, maxTurns: input.maxTurns,
        maxBudgetUsd: input.maxBudgetUsd,
        permissionMode: input.validateOnly ? 'default' : input.permissionMode || 'default',
        abortController: controller,
        mcpServers: input.validateOnly ? {} : { ...(input.mcpServers || {}), 'linggen-display': hostTools },
        hooks: {
          SubagentStart: [{ hooks: [subagents.start] }],
          SubagentStop: [{ hooks: [subagents.stop] }],
          UserPromptSubmit: [{ hooks: [turnContext(input.turnMemory)] }],
          PreToolUse: [{ hooks: [observeTool(send)] }],
          PostToolUse: [{ hooks: [endTool] }],
          PostToolUseFailure: [{ hooks: [endTool] }],
        },
        canUseTool: async (tool, args, options) => {
          const result = await permissionRequest(tool, args, rpc, { ...options, changeMode });
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
        send({ type: 'result', success: m.subtype === 'success' && !m.is_error,
          text: m.result, errors: m.errors, cost: m.total_cost_usd,
          session_id: m.session_id, usage: m.usage });
        finishInput!();
      } else if (m.type === 'system' && m.subtype === 'task_notification') {
        send({ type: 'text', text: m.summary || `Task ${m.task_id}: ${m.status}` });
      }
    }
    send({ type: 'done' });
  } catch (error) {
    send({ type: 'error', message: error instanceof Error ? error.message : String(error) });
    process.exitCode = 1;
  } finally {
    finishInput!();
    lines.close();
    for (const item of pending.values()) item.resolve({ allow: false });
  }
}

if (import.meta.main) await main();
