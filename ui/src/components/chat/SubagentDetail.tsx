/** One subagent's chat in the pane's detail view, rendered like the main
 *  chat: user bubble (task) → tool blocks → assistant text (result). Tool
 *  calls also show in the parent's main-chat tree, but here they use the
 *  same ⏺/⎿/✓ ContentBlockView as main chat (markdown, args/output
 *  expansion, etc.) so the pane IS the full thread between main agent and
 *  subagent. */
import React from 'react';
import { ArrowDown } from 'lucide-react';
import type { ContentBlock, PendingAskUser, AskUserAnswer, SubagentTreeEntry } from '../../types';
import { MarkdownContent } from './MarkdownContent';
import { ContentBlockView } from './ContentBlockView';
import { AskUserCard } from '../AskUserCard';
import { ToolPermissionCard } from '../ToolPermissionCard';
import { useAutoScroll } from '../../hooks/useAutoScroll';

export const SubagentDetail: React.FC<{
  entry: SubagentTreeEntry;
  /** The pending question, only when this subagent asked it. */
  pendingAskUser?: PendingAskUser | null;
  onRespondToAskUser?: (questionId: string, answers: AskUserAnswer[]) => void;
}> = ({ entry, pendingAskUser, onRespondToAskUser }) => {
  // Auto-scroll the same way the main chat does: follow tool calls and the
  // final result as they stream in, cancel when the user scrolls up, resume
  // when they scroll back to bottom. Tool-step count drives the
  // message-length signal; currentActivity + resultText drive the
  // streaming-text growth signal. The pane mounts this keyed by subagent,
  // so the hook resets cleanly when another one is opened.
  const { chatEndRef, showScrollButton, scrollToBottom } = useAutoScroll(
    { length: entry.toolSteps?.length ?? 0 },
    {
      isGenerating: entry.status === 'running',
      text: entry.resultText || '',
      liveText: entry.currentActivity || '',
    },
  );
  const name = entry.agentName || entry.subagentId;

  return (
    <div className="flex-1 overflow-y-auto px-3 py-3 custom-scrollbar min-h-0 flex flex-col gap-3 relative">
      {/* 1. Incoming task — user-style bubble. MarkdownContent handles the
       *    code blocks / lists / inline code in encoder prompts. */}
      {entry.task && (
        <div className="self-start max-w-[92%]">
          <div className="text-[10px] uppercase tracking-wider text-slate-400 dark:text-slate-500 font-semibold mb-1">
            From main → {name}
          </div>
          <div className="bg-slate-100 dark:bg-white/10 text-slate-900 dark:text-slate-100 rounded-md px-2.5 py-1.5 text-[13px]">
            <MarkdownContent text={entry.task} />
          </div>
        </div>
      )}

      {/* 2. Tool calls — same ContentBlockView main chat uses, so the
       *    bubble style, expansion, args/output render are identical. */}
      {entry.toolSteps && entry.toolSteps.length > 0 && (
        <div className="flex flex-col gap-1">
          {entry.toolSteps.map((step, idx) => {
            const block: ContentBlock = {
              type: 'tool_use',
              tool: step.toolName,
              args: step.args,
              status: step.status,
            };
            return (
              <ContentBlockView
                key={idx}
                block={block}
                isLast={idx === entry.toolSteps.length - 1}
              />
            );
          })}
        </div>
      )}

      {/* 3. Live thinking indicator while the subagent is mid-call
       *    with no tool yet — matches main chat's "Thinking…" line. */}
      {entry.status === 'running' &&
        (!entry.toolSteps || entry.toolSteps.length === 0) &&
        entry.currentActivity && (
          <div className="flex items-center gap-1.5 text-[12px] text-slate-500 dark:text-slate-400 italic">
            <span className="text-blue-500 animate-pulse">✶</span>
            <span>{entry.currentActivity}</span>
          </div>
        )}

      {/* 4. AskUser widget — same card main chat uses. */}
      {pendingAskUser && onRespondToAskUser && (
        <div>
          {pendingAskUser.questions[0]?.header === 'Permission' ? (
            <ToolPermissionCard pending={pendingAskUser} onRespond={onRespondToAskUser} />
          ) : (
            <AskUserCard pending={pendingAskUser} onRespond={onRespondToAskUser} />
          )}
        </div>
      )}

      {/* 5. Final result — assistant-style text bubble (left-aligned,
       *    same MD treatment as main chat agent replies). */}
      {entry.resultText && (
        <div className="self-start max-w-[92%]">
          <div className="text-[10px] uppercase tracking-wider text-slate-400 dark:text-slate-500 font-semibold mb-1">
            {name} → main
          </div>
          <div className="text-slate-800 dark:text-slate-200 text-[13px]">
            <MarkdownContent text={entry.resultText} />
          </div>
        </div>
      )}
      {/* Sentinel for auto-scroll-to-bottom — kept as the last child of
       *  the scroll container so the hook's scrollIntoView targets it. */}
      <div ref={chatEndRef} />
      {showScrollButton && (
        <button
          onClick={scrollToBottom}
          className="sticky bottom-2 left-1/2 -translate-x-1/2 z-30 w-8 h-8 rounded-full bg-white dark:bg-[#1a1a1a] border border-slate-300 dark:border-white/15 shadow-lg flex items-center justify-center hover:bg-slate-50 dark:hover:bg-white/10 transition-all opacity-80 hover:opacity-100"
          title="Scroll to bottom"
          aria-label="Scroll to bottom"
        >
          <ArrowDown size={14} className="text-slate-600 dark:text-slate-300" />
        </button>
      )}
    </div>
  );
};
