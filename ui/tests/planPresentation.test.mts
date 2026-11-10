import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isChecklistOnly, planTitle } from '../src/components/chat/utils/planPresentation.mts';

test('checklist-only plans have a plain title and recognize in-progress markdown', () => {
  const plan = {
    summary: '- [x] Inspect changes',
    plan_text: '- [x] Inspect changes\n- [~] Run tests *(in progress)*',
    items: [{ title: 'Inspect changes', status: 'completed' }, { title: 'Run tests', status: 'in_progress' }],
  };
  assert.equal(isChecklistOnly(plan.plan_text), true);
  assert.equal(planTitle(plan), 'Run tests');
  assert.equal(planTitle({ ...plan, items: [] }), 'Plan');
});

test('real plan text is not mistaken for a generated checklist', () => {
  assert.equal(isChecklistOnly('Context\n- [ ] Run tests'), false);
  assert.equal(planTitle({ summary: '# Ship safely', plan_text: 'Context' }), 'Ship safely');
});
