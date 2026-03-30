import React from 'react';
import { MarkdownContent } from './MarkdownContent';
import { PlanBlock } from './PlanBlock';
import { PLAN_STATUS_COLOR } from './utils/plan';
import type { SpecialBlockProps } from './types';

export function tryRenderSpecialBlock(
  text: string,
  props: SpecialBlockProps,
): React.ReactNode | null {
  try {
    const parsed = JSON.parse(text);

    if (parsed.type === 'plan' && parsed.plan) {
      return (
        <PlanBlock
          plan={parsed.plan}
          statusColor={PLAN_STATUS_COLOR}
          pendingPlanAgentId={props.pendingPlanAgentId}
          agentContext={props.agentContext}
          onApprovePlan={props.onApprovePlan}
          onRejectPlan={props.onRejectPlan}
          onEditPlan={props.onEditPlan}
          inputRef={props.inputRef}
        />
      );
    }

    if (parsed.type === 'finalize_task' && parsed.packet) {
      const packet = parsed.packet;
      const userStories: string[] = Array.isArray(packet.user_stories) ? packet.user_stories : [];
      const criteria: string[] = Array.isArray(packet.acceptance_criteria)
        ? packet.acceptance_criteria
        : [];
      return (
        <div className="space-y-2">
          <div className="font-bold text-blue-500">Task Finalized: {packet.title}</div>
          {userStories.length > 0 && (
            <div className="space-y-1 text-[12px]">
              <div className="uppercase tracking-wider text-[10px] text-slate-500">User Stories</div>
              {userStories.map((story: string, idx: number) => (
                <div key={idx} className="text-[12px] opacity-90">- {story}</div>
              ))}
            </div>
          )}
          {criteria.length > 0 && (
            <div className="space-y-1 text-[12px]">
              <div className="uppercase tracking-wider text-[10px] text-slate-500">Acceptance Criteria</div>
              {criteria.map((crit: string, idx: number) => (
                <div key={idx} className="text-[12px] opacity-90">- {crit}</div>
              ))}
            </div>
          )}
        </div>
      );
    }

    // JSON with text-like field — extract and render as markdown
    const textContent =
      typeof parsed.response === 'string' ? parsed.response
      : typeof parsed.text === 'string' ? parsed.text
      : typeof parsed.content === 'string' ? parsed.content
      : typeof parsed.message === 'string' ? parsed.message
      : typeof parsed.answer === 'string' ? parsed.answer
      : null;
    if (textContent) return <MarkdownContent text={textContent} />;
  } catch { /* not JSON */ }
  return null;
}
