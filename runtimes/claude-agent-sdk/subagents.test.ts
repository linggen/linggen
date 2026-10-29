import { expect, test } from 'bun:test';
import { contextTokens, subagentEvents, taskEvent } from './subagents';

test('parallel children keep separate identities and never become main-thread text', () => {
  const events: any[] = []; const bridge = subagentEvents(e => events.push(e));
  for (const id of ['a', 'b']) bridge.route({ type:'system', subtype:'task_started', task_type:'local_agent', task_id:id, tool_use_id:`tool-${id}`, subagent_type:'reviewer', prompt:`Review ${id}` });
  expect(bridge.route({type:'assistant', parent_tool_use_id:'tool-a', message:{content:[{type:'text', text:'child A'}]}})).toBe(true);
  expect(bridge.route({type:'assistant', agent_id:'b', parent_tool_use_id:'tool-b', message:{content:[{type:'text', text:'child B'}]}})).toBe(true);
  expect(bridge.route({type:'stream_event', parent_tool_use_id:'tool-a', event:{delta:{type:'text_delta', text:'private'}}})).toBe(true);
  expect(bridge.route({type:'assistant', parent_tool_use_id:null, message:{content:[{type:'text', text:'main'}]}})).toBe(false);
  expect(events.filter(e => e.type === 'subagent_text')).toEqual([
    {type:'subagent_text', agent_id:'a', text:'child A'}, {type:'subagent_text', agent_id:'b', text:'child B'},
  ]);
  expect(bridge.route({type:'system', subtype:'task_notification', task_id:'a', status:'completed', summary:'A done'})).toBe(true);
  expect(events.at(-1).type).toBe('subagent_end');
});

test('unknown child frames stay out of main chat even before task registration', () => {
  const events: any[] = []; const bridge = subagentEvents(e => events.push(e));
  expect(bridge.route({type:'assistant', parent_tool_use_id:'early', message:{content:[{type:'text', text:'private'}]}})).toBe(true);
  expect(events.at(-1)).toMatchObject({type:'subagent_text', text:'private'});
  expect(bridge.route({type:'system', subtype:'task_notification', task_id:'bash', summary:'shell done'})).toBe(false);
});

test('the SDK compaction summarizer is no child', async () => {
  const events: any[] = []; const bridge = subagentEvents(e => events.push(e));
  await bridge.start({ hook_event_name:'SubagentStart', agent_id:'compact', agent_type:'' });
  await bridge.stop({ hook_event_name:'SubagentStop', agent_id:'compact', agent_type:'', last_assistant_message:'<analysis>\nsummary' });
  expect(events).toEqual([]);
  await bridge.start({ hook_event_name:'SubagentStart', agent_id:'real', agent_type:'general-purpose' });
  await bridge.stop({ hook_event_name:'SubagentStop', agent_id:'real', agent_type:'general-purpose', last_assistant_message:'done' });
  expect(events.map(e => e.type)).toEqual(['subagent_start', 'subagent_end']);
});

test('background task frames become the session task list', () => {
  expect(taskEvent({ type:'system', subtype:'background_tasks_changed', tasks:[{ task_id:'t1', task_type:'local_bash', description:'npm test' }] }))
    .toEqual({ type:'tasks', tasks:[{ id:'t1', kind:'shell', title:'npm test' }] });
  expect(taskEvent({ type:'system', subtype:'task_started', task_id:'t2', task_type:'local_agent', description:'review', is_backgrounded:true }))
    .toEqual({ type:'task', id:'t2', kind:'subagent', title:'review', background:true });
  expect(taskEvent({ type:'system', subtype:'task_notification', task_id:'t1', status:'failed', summary:'2 failed', output_file:'/tmp/o' }))
    .toEqual({ type:'task', id:'t1', status:'failed', summary:'2 failed', output_file:'/tmp/o' });
  expect(taskEvent({ type:'assistant' })).toBeNull();
});

test('a child that hands back its report gives that report as its result', () => {
  const events: any[] = []; const bridge = subagentEvents(e => events.push(e));
  bridge.route({ type:'system', subtype:'task_started', task_type:'local_agent', task_id:'c', tool_use_id:'tool-c', subagent_type:'general-purpose', prompt:'Fix it' });
  bridge.route({ type:'assistant', agent_id:'c', parent_tool_use_id:'tool-c', message:{ content:[{ type:'text', text:'Mobile done. Now engine tests' }] } });
  expect(bridge.route({ type:'assistant', agent_id:'c', parent_tool_use_id:'tool-c', message:{ content:[
    { type:'tool_use', id:'h1', name:'SubagentHandback', input:{ message:'## Fixed\nAll tests pass.' } },
  ] } })).toBe(true);
  expect(events.filter(e => e.type === 'subagent_report')).toEqual([{ type:'subagent_report', agent_id:'c', text:'## Fixed\nAll tests pass.' }]);
  expect(taskEvent({ type:'system', subtype:'task_notification', task_id:'c', status:'completed',
    summary:"This agent's report was delivered to you as a message from c" })).toMatchObject({ summary:'' });
});

test("a child's own background shell is the child's: its end never reaches the main thread", () => {
  const events: any[] = []; const bridge = subagentEvents(e => events.push(e));
  bridge.route({ type:'system', subtype:'task_started', task_type:'local_agent', task_id:'c', tool_use_id:'tool-c', subagent_type:'general-purpose', prompt:'Build it' });
  expect(bridge.route({ type:'system', subtype:'task_started', task_type:'local_bash', task_id:'b1', parent_task_id:'c', description:'cargo test', is_backgrounded:true })).toBe(true);
  expect(bridge.route({ type:'system', subtype:'task_progress', task_id:'b1', description:'cargo test', usage:{} })).toBe(true);
  expect(bridge.route({ type:'system', subtype:'task_notification', task_id:'b1', status:'completed', summary:'cargo test' })).toBe(true);
  expect(events.map(e => e.type)).toEqual(['subagent_start']);
  // A shell the main thread launched is not a child's.
  expect(bridge.route({ type:'system', subtype:'task_started', task_type:'local_bash', task_id:'b2', description:'npm test' })).toBe(false);
  expect(bridge.route({ type:'system', subtype:'task_notification', task_id:'b2', status:'completed', summary:'npm test' })).toBe(false);
});

test("a child's context is its latest request's whole prompt, sent when it changes", () => {
  const events: any[] = []; const bridge = subagentEvents(e => events.push(e));
  bridge.route({ type:'system', subtype:'task_started', task_type:'local_agent', task_id:'c', tool_use_id:'tool-c', subagent_type:'general-purpose', prompt:'Fix it' });
  const usage = { input_tokens:12, cache_read_input_tokens:40000, cache_creation_input_tokens:5000, output_tokens:300 };
  const frame = (content: any[]) => ({ type:'assistant', agent_id:'c', parent_tool_use_id:'tool-c', message:{ content, usage } });
  bridge.route(frame([{ type:'thinking', thinking:'…' }]));
  bridge.route(frame([{ type:'text', text:'Reading' }]));
  expect(events.filter(e => e.type === 'subagent_context')).toEqual([{ type:'subagent_context', agent_id:'c', tokens:45012 }]);
  expect(contextTokens(undefined)).toBe(0);
});
