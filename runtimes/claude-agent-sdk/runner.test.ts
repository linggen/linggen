import { describe, expect, test } from 'bun:test';
import { permissionHook, userQuestion, queryPrompt } from './runner';

describe('Linggen permission bridge', () => {
  test('every call reaches the host, including SDK read tools', async () => {
    const calls: any[] = [];
    const hook = permissionHook(async (kind, input) => {
      calls.push({ kind, input });
      return { allow: true };
    }, () => {});
    const result = await hook({ tool_name: 'Read', tool_input: { file_path: '/project/a' }, tool_use_id: 'one' });
    expect(calls[0].kind).toBe('permission');
    expect(result.hookSpecificOutput?.permissionDecision).toBe('allow');
  });

  test('a denial closes the tool and cannot fall through to SDK auto-approval', async () => {
    const events: any[] = [];
    const hook = permissionHook(async () => ({ allow: false, reason: 'Outside read grant' }), event => events.push(event));
    const result = await hook({ tool_name: 'Bash', tool_input: { command: 'rm file' }, tool_use_id: 'two' });
    expect(result.hookSpecificOutput?.permissionDecision).toBe('deny');
    expect(events.map(event => event.type)).toEqual(['tool_start', 'tool_end']);
    expect(events[1].failed).toBe(true);
  });

  test('questions reach the host UI and carry its answers back to the SDK', async () => {
    const hook = permissionHook(async () => ({ allow: true }), () => {});
    expect(await hook({ tool_name: 'AskUserQuestion', tool_input: {}, tool_use_id: 'three' })).toEqual({});
    const result = await userQuestion('AskUserQuestion', { questions: [{ question: 'Which?' }] },
      async () => ({ answers: { 'Which?': 'First' } }));
    expect(result).toEqual({ behavior: 'allow', updatedInput: {
      questions: [{ question: 'Which?' }], answers: { 'Which?': 'First' },
    } });
  });

  test('unexpected permission callbacks fail closed', async () => {
    const result = await userQuestion('Bash', { command: 'echo x' }, async () => { throw new Error('must not call'); });
    expect(result.behavior).toBe('deny');
  });

  test('a plan must be approved through the host', async () => {
    const hook = permissionHook(async () => ({ allow: true }), () => {});
    expect(await hook({ tool_name: 'ExitPlanMode', tool_input: {}, tool_use_id: 'plan' })).toEqual({});
    const result = await userQuestion('ExitPlanMode', {}, async () => ({ allow: false }));
    expect(result.behavior).toBe('deny');
  });

  test('host-stamped MCP context is passed to the SDK tool', async () => {
    const hook = permissionHook(async () => ({ allow: true, updatedInput: { query: 'x', scope_root: '/project' } }), () => {});
    const result = await hook({ tool_name: 'mcp__memory__memory_search', tool_input: { query: 'x' }, tool_use_id: 'memory' });
    expect(result.hookSpecificOutput?.updatedInput).toEqual({ query: 'x', scope_root: '/project' });
  });

  test('images reach the SDK as content blocks', async () => {
    const prompt = queryPrompt({ prompt: 'Inspect', images: ['data:image/png;base64,YWJj'] });
    const messages: any[] = [];
    for await (const message of prompt) messages.push(message);
    expect(messages[0].message.content[1]).toEqual({ type: 'image', source: { type: 'base64', media_type: 'image/png', data: 'YWJj' } });
  });
});
