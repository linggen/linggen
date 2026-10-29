import { expect, test } from 'bun:test';
import { subagentEvents, taskEvent } from './subagents';

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

test('background task frames become the session task list', () => {
  expect(taskEvent({ type:'system', subtype:'background_tasks_changed', tasks:[{ task_id:'t1', task_type:'local_bash', description:'npm test' }] }))
    .toEqual({ type:'tasks', tasks:[{ id:'t1', kind:'shell', title:'npm test' }] });
  expect(taskEvent({ type:'system', subtype:'task_started', task_id:'t2', task_type:'local_agent', description:'review', is_backgrounded:true }))
    .toEqual({ type:'task', id:'t2', kind:'subagent', title:'review', background:true });
  expect(taskEvent({ type:'system', subtype:'task_notification', task_id:'t1', status:'failed', summary:'2 failed', output_file:'/tmp/o' }))
    .toEqual({ type:'task', id:'t1', status:'failed', summary:'2 failed', output_file:'/tmp/o' });
  expect(taskEvent({ type:'assistant' })).toBeNull();
});
