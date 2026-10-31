import { describe, expect, test } from 'bun:test';
import { followCwd, modeSettles, observeTool, staleAsk, permissionRequest, queryPrompt, userMessage, inbox, sessionPermissionSettings, settingSources, turnContext } from './runner';

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
      tool: 'Bash', args, title: 'Run tests', reason: 'SDK needs approval', description: undefined, suggestions: [],
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
    for await (const message of queryPrompt(userMessage({ prompt: 'Inspect', images: ['data:image/png;base64,YWJj'] }))) messages.push(message);
    expect(messages[0].message.content[1]).toEqual({ type: 'image', source: { type: 'base64', media_type: 'image/png', data: 'YWJj' } });
    expect(messages[0].origin).toEqual({ kind: 'human' });
    expect(messages[0].priority).toBeUndefined();
  });

  test('a message sent now joins the running turn after its first, then input ends', async () => {
    const later = inbox<any>();
    const seen: any[] = [];
    const reading = (async () => {
      for await (const message of queryPrompt(userMessage({ prompt: 'Build it' }), later)) seen.push(message);
    })();
    later.push(userMessage({ prompt: 'Skip the tests' }, 'now'));
    later.close();
    await reading;
    expect(seen.map((m) => m.message.content[0].text)).toEqual(['Build it', 'Skip the tests']);
    expect(seen[1].priority).toBe('now');
    expect(later.closed).toBe(true);
  });

  test('session approval returns SDK suggestions without writing global settings', async () => {
    const suggestions = [{ type:'addRules', rules:[{toolName:'Bash',ruleContent:'bun test'}], behavior:'allow', destination:'userSettings' }];
    const result = await permissionRequest('Bash', {command:'bun test'}, async (kind) => kind === 'permission' ? ({allow:true,remember:true}) : ({saved:true}), {suggestions});
    expect(result.updatedPermissions).toEqual([{...suggestions[0],destination:'session'}]);
    expect(suggestions[0].destination).toBe('userSettings');
  });

  test('unsupported auto mode does not approve the pending operation', async () => {
    const result = await permissionRequest('Edit', {}, async () => ({allow:true,mode:'auto'}), {
      changeMode: async () => { throw new Error('auto unavailable'); },
    });
    expect(result).toEqual({behavior:'deny',message:'Error: auto unavailable'});
  });

  test('mode selection is applied before the user-approved call resumes', async () => {
    const modes:string[]=[];
    const result = await permissionRequest('Edit', {}, async () => ({allow:true,mode:'auto'}), {changeMode:async(mode:string)=>{modes.push(mode);}});
    expect(modes).toEqual(['auto']);
    expect(result.behavior).toBe('allow');
  });
});

 test('SDK permission updates rehydrate as session settings, including removals',()=> {
 const settings=sessionPermissionSettings([
 {type:'addRules',behavior:'allow',rules:[{toolName:'Bash',ruleContent:'bun test *'}]},
 {type:'addRules',behavior:'allow',rules:[{toolName:'Edit'}]},
 {type:'removeRules',behavior:'allow',rules:[{toolName:'Edit'}]},
 {type:'addDirectories',directories:['/work']},
 ]);
 expect(settings.permissions.allow).toEqual(['Bash(bun test *)']);
 expect(settings.permissions.additionalDirectories).toEqual(['/work']);
 });

test('remembering a tool does not silently upgrade the SDK permission mode', async () => {
  const result = await permissionRequest('Bash', {}, async (kind) => kind === 'permission' ? {allow:true,remember:true} : {saved:true}, {
    suggestions:[{type:'addRules',behavior:'allow',rules:[{toolName:'Bash',ruleContent:'touch example'}]}, {type:'setMode',mode:'acceptEdits',destination:'session'}],
  });
  expect(result.updatedPermissions).toHaveLength(1);
  expect(result.updatedPermissions![0].type).toBe('addRules');
});

describe('mode changes and folder trust', () => {
  test('a mode change withdraws only the prompts the new mode decides itself', () => {
    expect(modeSettles('acceptEdits', 'Edit')).toBe(true);
    expect(modeSettles('acceptEdits', 'Bash')).toBe(false);
    expect(modeSettles('auto', 'Bash')).toBe(true);
    expect(modeSettles('dontAsk', 'Bash')).toBe(true);
    expect(modeSettles('default', 'Edit')).toBe(false);
    expect(modeSettles('plan', 'Bash')).toBe(false);
  });

  test('an ask decided before the mode change is withdrawn; one decided after it stays', () => {
    expect(staleAsk(0, 1, 'auto', 'Bash')).toBe(true);
    expect(staleAsk(1, 1, 'auto', 'Bash')).toBe(false);
    expect(staleAsk(0, 1, 'acceptEdits', 'Bash')).toBe(false);
    expect(staleAsk(undefined, 1, 'auto', 'Bash')).toBe(false);
  });

  test('project and local settings load only when the host says the folder is trusted', () => {
    expect(settingSources({})).toEqual(['user']);
    expect(settingSources({ settingSources: ['user', 'project', 'local'] })).toEqual(['user', 'project', 'local']);
  });
});

test('each message\'s turn memory reaches the SDK as prompt context, in order, and nothing when empty', async () => {
  const memories = ['From memory: tabs', '', 'From memory: dark mode'];
  const hook = turnContext(memories);
  expect(await hook()).toEqual({
    hookSpecificOutput: { hookEventName: 'UserPromptSubmit', additionalContext: 'From memory: tabs' } });
  expect(await hook()).toEqual({});
  expect(((await hook()) as any).hookSpecificOutput.additionalContext).toBe('From memory: dark mode');
  expect(await hook()).toEqual({});
});

test('a later message carries its own uuid, a steered one priority now', () => {
  expect(userMessage({ prompt: 'next' }, undefined, 'u2').uuid).toBe('u2');
  const steer = userMessage({ prompt: 'stop' }, 'now');
  expect(steer.priority).toBe('now');
  expect((steer as any).uuid).toBeUndefined();
  // Typed while Claude worked, delivered at a tool round: folded in next.
  expect(userMessage({ prompt: 'also the docs' }, 'next').priority).toBe('next');
});

test('the main thread\'s cd, read off tool hooks, reaches the host; a subagent\'s does not', () => {
  const events: any[] = [];
  const follow = followCwd((event) => events.push(event));
  follow({ hook_event_name: 'PreToolUse', cwd: '/repo' });
  follow({ hook_event_name: 'PreToolUse', cwd: '/tmp', agent_id: 'sub-1' });
  follow({ hook_event_name: 'PostToolUse', cwd: '/tmp', agent_id: 'sub-1' });
  expect(events).toEqual([]);
  follow({ hook_event_name: 'PostToolUse', cwd: '/repo/src' });
  follow({ hook_event_name: 'PreToolUse', cwd: '/repo/src' });
  expect(events).toEqual([{ type: 'cwd', cwd: '/repo/src' }]);
});
