// JSON-lines bridge. Only stdout carries protocol; credentials stay in env.
import { createInterface } from 'node:readline';

type Send = (message: Record<string, unknown>) => void;
type Rpc = (kind: string, input: any) => Promise<any>;

// Observe calls without making permission decisions. The SDK's own rules and
// modes run before canUseTool asks the host to render a widget.
export function observeTool(send: Send) {
  return async (input: any) => {
    send({ type: 'tool_start', tool: input.tool_name, args: input.tool_input, id: input.tool_use_id });
    return {};
  };
}

export async function permissionRequest(tool: string, input: any, rpc: Rpc, options: any = {}) {
  if (tool === 'AskUserQuestion') {
    const response = await rpc('ask_user', input);
    if (!response.answers) return { behavior: 'deny' as const, message: 'Question cancelled.' };
    return { behavior: 'allow' as const, updatedInput: { ...input, answers: response.answers } };
  }
  const response = await rpc('permission', { tool, args: input,
    title: options.title, description: options.description, reason: options.decisionReason });
  return response.allow ? { behavior: 'allow' as const, updatedInput: input }
    : { behavior: 'deny' as const, message: response.reason || 'User declined this tool call.' };
}

export function queryPrompt(input: any) {
  if (!input.images?.length) return input.prompt;
  const content: any[] = [{ type: 'text', text: input.prompt }];
  for (const image of input.images) {
    const match = /^data:(image\/[\w.+-]+);base64,(.+)$/s.exec(image);
    if (!match) throw new Error('Expected a base64 image data URL');
    content.push({ type: 'image', source: { type: 'base64', media_type: match[1], data: match[2] } });
  }
  return (async function* () {
    yield { type: 'user' as const, session_id: '', parent_tool_use_id: null,
      message: { role: 'user' as const, content } };
  })();
}

async function main() {
  const { query } = await import('@anthropic-ai/claude-agent-sdk');
  const send: Send = message => process.stdout.write(`${JSON.stringify(message)}\n`);
  const controller = new AbortController();
  const pending = new Map<number, (response: any) => void>();
  let seq = 0;
  let start: (input: any) => void;
  const config = new Promise<any>(resolve => { start = resolve; });
  const lines = createInterface({ input: process.stdin });
  lines.on('line', line => {
    const message = JSON.parse(line);
    if (message.type === 'start') start(message);
    if (message.type === 'reply') {
      pending.get(message.id)?.(message);
      pending.delete(message.id);
    }
    if (message.type === 'cancel') controller.abort();
  });
  lines.on('close', () => controller.abort());
  const rpc: Rpc = (kind, input) => new Promise(resolve => {
    const id = ++seq;
    pending.set(id, resolve);
    send({ type: 'request', kind, id, input });
  });
  const input = await config;
  const env = { ...process.env };
  for (const key of ['ANTHROPIC_AUTH_TOKEN', 'CLAUDE_CODE_OAUTH_TOKEN', 'CLAUDECODE',
    'CLAUDE_CODE_USE_BEDROCK', 'CLAUDE_CODE_USE_VERTEX', 'CLAUDE_CODE_USE_FOUNDRY']) delete env[key];
  env.CLAUDE_AGENT_SDK_CLIENT_APP = 'linggen';
  const endTool = async (event: any) => {
    send({ type: 'tool_end', id: event.tool_use_id, tool: event.tool_name,
      failed: event.hook_event_name === 'PostToolUseFailure',
      result: event.tool_response ?? event.error ?? '' });
    return {};
  };
  try {
    const conversation = query({
      prompt: queryPrompt(input),
      options: {
        cwd: input.cwd, model: input.model, env, settingSources: ['project'],
        resume: input.resume || undefined,
        systemPrompt: { type: 'preset', preset: 'claude_code' },
        tools: { type: 'preset', preset: 'claude_code' },
        disallowedTools: input.disallowedTools || [],
        includePartialMessages: true, maxTurns: input.maxTurns,
        maxBudgetUsd: input.maxBudgetUsd,
        permissionMode: input.permissionMode || 'default',
        abortController: controller,
        mcpServers: input.mcpServers || {},
        hooks: {
          PreToolUse: [{ hooks: [observeTool(send)] }],
          PostToolUse: [{ hooks: [endTool] }],
          PostToolUseFailure: [{ hooks: [endTool] }],
        },
        canUseTool: async (tool, args, options) => {
          const result = await permissionRequest(tool, args, rpc, options);
          if (result.behavior === 'deny') send({ type: 'tool_end', id: options.toolUseID,
            tool, failed: true, result: result.message });
          return result;
        },
      },
    });
    for await (const message of conversation) {
      const m = message as any;
      if (m.type === 'system' && m.subtype === 'init') {
        send({ type: 'init', session_id: m.session_id, model: m.model });
      } else if (m.type === 'system' && m.subtype === 'status') {
        send({ type: 'status', status: m.status, permissionMode: m.permissionMode });
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
      }
    }
  } catch (error) {
    send({ type: 'error', message: error instanceof Error ? error.message : String(error) });
    process.exitCode = 1;
  } finally {
    lines.close();
    for (const resolve of pending.values()) resolve({ allow: false });
  }
}

if (import.meta.main) await main();
