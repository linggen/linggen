import { describe, expect, test } from 'bun:test';
import { observeTool, permissionRequest, queryPrompt } from './runner';

describe('SDK-owned permissions with Linggen widgets', () => {
  test('observing a tool cannot approve or deny it', async () => {
    const events: any[] = [];
    const result = await observeTool((event) => events.push(event))({
      tool_name: 'Read', tool_input: { file_path: '/project/a' }, tool_use_id: 'one',
    });
    expect(result).toEqual({});
    expect(events).toEqual([{ type: 'tool_start', tool: 'Read', args: { file_path: '/project/a' }, id: 'one' }]);
  });

  test('SDK approval requests return the user decision with the original input', async () => {
    const args = { command: 'bun test' };
    const calls: any[] = [];
    const result = await permissionRequest('Bash', args, async (kind, input) => {
      calls.push({ kind, input }); return { allow: true };
    }, { title: 'Run tests', decisionReason: 'SDK needs approval' });
    expect(calls).toEqual([{ kind: 'permission', input: {
      tool: 'Bash', args, title: 'Run tests', reason: 'SDK needs approval', description: undefined,
    } }]);
    expect(result).toEqual({ behavior: 'allow', updatedInput: args });
  });

  test('denial carries the user explanation back to the SDK', async () => {
    const result = await permissionRequest('Edit', {}, async () => ({ allow: false, reason: 'Leave that file alone.' }));
    expect(result).toEqual({ behavior: 'deny', message: 'Leave that file alone.' });
  });

  test('AskUserQuestion maps widget answers back without a native AskUser call', async () => {
    const result = await permissionRequest('AskUserQuestion', { questions: [{ question: 'Which?' }] },
      async (kind) => { expect(kind).toBe('ask_user'); return { answers: { 'Which?': 'Alpha' } }; });
    expect(result).toEqual({ behavior: 'allow', updatedInput: {
      questions: [{ question: 'Which?' }], answers: { 'Which?': 'Alpha' },
    } });
  });

  test('plan approval follows the same SDK approval callback', async () => {
    const result = await permissionRequest('ExitPlanMode', {}, async (kind) => {
      expect(kind).toBe('permission'); return { allow: false };
    });
    expect(result.behavior).toBe('deny');
  });

  test('images reach the SDK as content blocks', async () => {
    const messages: any[] = [];
    for await (const message of queryPrompt({ prompt: 'Inspect', images: ['data:image/png;base64,YWJj'] })) messages.push(message);
    expect(messages[0].message.content[1]).toEqual({ type: 'image', source: { type: 'base64', media_type: 'image/png', data: 'YWJj' } });
  });
});
