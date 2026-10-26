import { describe, expect, test } from 'bun:test';
import { observeTool, permissionRequest, queryPrompt, sessionPermissionSettings, turnContext } from './runner';

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
    for await (const message of queryPrompt({ prompt: 'Inspect', images: ['data:image/png;base64,YWJj'] })) messages.push(message);
    expect(messages[0].message.content[1]).toEqual({ type: 'image', source: { type: 'base64', media_type: 'image/png', data: 'YWJj' } });
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

test('the host\'s turn memory reaches the SDK as prompt context, and nothing when empty', async () => {
  expect(await turnContext('From memory: tabs')()).toEqual({
    hookSpecificOutput: { hookEventName: 'UserPromptSubmit', additionalContext: 'From memory: tabs' } });
  expect(await turnContext('')()).toEqual({});
  expect(await turnContext(undefined)()).toEqual({});
});
