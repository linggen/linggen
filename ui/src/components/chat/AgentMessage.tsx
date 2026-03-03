import React from 'react';
import type { ChatMessage, ContentBlock } from '../../types';
import type { SpecialBlockProps } from './types';
import { MarkdownContent } from './MarkdownContent';
import { ThinkingIndicator, StatusSpinner } from './ThinkingIndicator';
import { SubagentTreeView } from './SubagentTreeView';
import { ContentBlockList, TurnSummaryFooter } from './ContentBlockView';
import { tryRenderSpecialBlock } from './SpecialBlocks';
import { getMessagePhase, isTransientStatus, isToolStatusText } from './MessagePhase';
import { visibleMessageText } from './MessageHelpers';

/** Top-level agent message renderer — unified widget-list model.
 *
 *  Primary path: renders from msg.content[] (ContentBlock array).
 *  Fallback: renders msg.text as markdown (for legacy/context messages without content blocks).
 *
 *  Styling is identical in generating and done states — the only difference is:
 *  - StatusSpinner appears during generation gaps
 *  - ContentBlockList auto-collapses when round ends
 *  - TurnSummaryFooter appears when done
 */
export const AgentMessage: React.FC<{
  msg: ChatMessage;
  isExpanded: boolean;
  onToggle: () => void;
  planProps: SpecialBlockProps;
}> = ({ msg, isExpanded, onToggle, planProps }) => {
  const phase = getMessagePhase(msg);
  const contentBlocks = msg.content || [];
  const hasToolBlocks = contentBlocks.some(b => b.type === 'tool_use');
  const hasRunningTools = contentBlocks.some(b => b.type === 'tool_use' && b.status === 'running');
  const isStreamingText = !!msg.liveText;

  const hasAnyActivity = hasToolBlocks ||
    (msg.activityEntries || []).some(e => !isTransientStatus(e));
  const showThinking = phase === 'thinking' && !hasAnyActivity;

  const showSpinner = !!msg.isGenerating && !hasRunningTools && !isStreamingText && !showThinking;

  const segments: Array<{ kind: 'text'; text: string } | { kind: 'tools'; blocks: ContentBlock[] }> = [];
  if (hasToolBlocks) {
    for (const block of contentBlocks) {
      if (block.type === 'text' && block.text) {
        if (!isToolStatusText(block.text)) {
          segments.push({ kind: 'text', text: block.text });
        }
      } else if (block.type === 'tool_use') {
        const last = segments[segments.length - 1];
        if (last && last.kind === 'tools') {
          last.blocks.push(block);
        } else {
          segments.push({ kind: 'tools', blocks: [block] });
        }
      }
    }
  }

  const fallbackText = !hasToolBlocks ? visibleMessageText(msg) : '';

  // Error messages get a prominent banner style.
  if (msg.isError) {
    const errorText = fallbackText || msg.text || 'An error occurred';
    return (
      <div className="rounded-lg border border-red-300 dark:border-red-700 bg-red-50 dark:bg-red-950/40 px-4 py-3 text-sm text-red-800 dark:text-red-300">
        <div className="flex items-start gap-2">
          <span className="mt-0.5 shrink-0 text-red-500 dark:text-red-400">&#x26A0;</span>
          <MarkdownContent text={errorText} />
        </div>
      </div>
    );
  }

  return (
    <>
      {msg.subagentTree && msg.subagentTree.length > 0 && (
        <SubagentTreeView
          entries={msg.subagentTree}
          isGenerating={!!msg.isGenerating}
          isExpanded={isExpanded}
          onToggle={onToggle}
        />
      )}

      {showThinking && <ThinkingIndicator text={msg.text || 'Thinking...'} />}

      {hasToolBlocks && segments.map((seg, idx) => {
        if (seg.kind === 'text') {
          const specialBlock = tryRenderSpecialBlock(seg.text, planProps);
          if (specialBlock) return <React.Fragment key={`seg-${idx}`}>{specialBlock}</React.Fragment>;
          return <MarkdownContent key={`seg-${idx}`} text={seg.text} />;
        }
        return (
          <ContentBlockList
            key={`seg-${idx}`}
            blocks={seg.blocks}
            isGenerating={!!msg.isGenerating}
          />
        );
      })}

      {!hasToolBlocks && !showThinking && fallbackText && !isTransientStatus(fallbackText) && (() => {
        const specialBlock = tryRenderSpecialBlock(fallbackText, planProps);
        if (specialBlock) return <>{specialBlock}</>;
        return <MarkdownContent text={fallbackText} />;
      })()}

      {isStreamingText && msg.liveText && (
        <>
          <MarkdownContent text={msg.liveText} />
          <span className="inline-block w-1.5 h-3.5 bg-blue-500 ml-1 animate-pulse align-middle" />
        </>
      )}

      {showSpinner && <StatusSpinner />}

      <TurnSummaryFooter msg={msg} />
    </>
  );
};
