type Item = { title: string; status: string };
type PlanView = { summary: string; plan_text: string; items?: Item[] };

const checklistLine = /^\s*[-*+]\s+\[[ xX~]\]\s+/;

/** Engine-generated checklist text is a representation of items, not plan prose. */
export function isChecklistOnly(text: string): boolean {
  const lines = text.split('\n').filter(line => line.trim());
  return lines.length > 0 && lines.every(line => checklistLine.test(line));
}

export function planTitle(plan: PlanView): string {
  const summary = plan.summary.trim();
  if (checklistLine.test(summary) || isChecklistOnly(plan.plan_text)) {
    const item = plan.items?.find(i => i.status !== 'completed' && i.status !== 'done');
    return item?.title || 'Plan';
  }
  return summary.replace(/^\s*#{1,6}\s+/, '').replace(/[*_`~]/g, '').trim() || 'Plan';
}
