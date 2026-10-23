// JSON-lines bridge. Only stdout carries protocol; credentials stay in env.
import { createInterface } from 'node:readline';

type Send = (message: Record<string, unknown>) => void;
type Rpc = (kind: string, input: any) => Promise<any>;

export function permissionHook(rpc: Rpc, send: Send) {
  return async (input: any) => {
    const { tool_name: tool, tool_input: args, tool_use_id: id } = input;
    send({ type: 'tool_start', tool, args, id });
    const response = await rpc('permission', input);
    if (!response.allow) {
      send({ type: 'tool_end', id, tool, failed: true, result: response.reason });
      return { hookSpecificOutput: {
        hookEventName: 'PreToolUse', permissionDecision: 'deny',
        permissionDecisionReason: response.reason || 'Linggen denied this tool call.',
      } };
    }
    // AskUserQuestion must reach canUseTool, where Linggen supplies answers.
    if (tool === 'AskUserQuestion' || tool === 'ExitPlanMode') return {};
    return { hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'allow',
      ...(response.updatedInput ? { updatedInput: response.updatedInput } : {}) } };
  };
}

export async function userQuestion(tool: string, input: any, rpc: Rpc) {
  if (tool === 'ExitPlanMode') {
    const response = await rpc('exit_plan', input);
    return response.allow ? { behavior: 'allow' as const, updatedInput: input }
      : { behavior: 'deny' as const, message: 'The plan was not approved.' };
  }
  if (tool !== 'AskUserQuestion') {
    return { behavior: 'deny' as const, message: 'Tool did not pass Linggen permission checks.' };
  }
  const response = await rpc('ask_user', input);
  if (!response.answers) return { behavior: 'deny' as const, message: 'Question cancelled.' };
  return { behavior: 'allow' as const, updatedInput: { ...input, answers: response.answers } };
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
        cwd: input.cwd, model: input.model, env, settingSources: [],
        resume: input.resume || undefined,
        systemPrompt: { type: 'preset', preset: 'claude_code',
          ...(input.append ? { append: input.append } : {}) },
        tools: !input.tools || input.tools.includes('*')
          ? { type: 'preset', preset: 'claude_code' } : input.tools,
        includePartialMessages: true, maxTurns: input.maxTurns,
        maxBudgetUsd: input.maxBudgetUsd,
        permissionMode: input.planMode ? 'plan' : 'default',
        abortController: controller,
        mcpServers: input.mcpServers || {},
        hooks: {
          PreToolUse: [{ hooks: [permissionHook(rpc, send)], timeout: 28800 }],
          PostToolUse: [{ hooks: [endTool] }],
          PostToolUseFailure: [{ hooks: [endTool] }],
        },
        canUseTool: (tool, args) => userQuestion(tool, args, rpc),
      },
    });
    for await (const message of conversation) {
      const m = message as any;
      if (m.type === 'system' && m.subtype === 'init') {
        send({ type: 'init', session_id: m.session_id, model: m.model });
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
