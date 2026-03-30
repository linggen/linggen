import React, { useEffect, useMemo, useRef, useState } from 'react';
import { cn } from '../../lib/cn';
import DiffView, { diffStats } from '../DiffView';
import type { ChatMessage, ContentBlock } from '../../types';
import { truncateDetail, contentBlockSummary, buildInlineDiff } from './utils/content-block';

/** Compact footer showing turn stats: tool calls, tokens, duration. */
export const TurnSummaryFooter: React.FC<{ msg: ChatMessage }> = ({ msg }) => {
  if (msg.isGenerating) return null;
  const tools = msg.toolCount || 0;
  const tokens = msg.contextTokens || 0;
  const durationMs = msg.durationMs || 0;
  if (tools === 0 && tokens === 0 && durationMs === 0) return null;

  const parts: string[] = [];
  if (tools > 0) parts.push(`${tools} tool${tools !== 1 ? 's' : ''}`);
  if (tokens > 0) {
    const k = tokens >= 1000 ? `${(tokens / 1000).toFixed(1)}k` : String(tokens);
    parts.push(`${k} tokens`);
  }
  if (durationMs > 0) {
    const sec = (durationMs / 1000).toFixed(1);
    parts.push(`${sec}s`);
  }
  if (parts.length === 0) return null;

  return (
    <div className="mt-1.5 flex items-center gap-1.5 text-[11px] font-mono text-slate-400 dark:text-slate-500 select-none">
      <span className="w-1 h-1 rounded-full bg-slate-300 dark:bg-slate-600" />
      {parts.join(' · ')}
    </div>
  );
};

/** Render a single ContentBlock as a compact tool line with optional Bash output / diff widget. */
export const ContentBlockView: React.FC<{
  block: ContentBlock;
  isLast: boolean;
}> = React.memo(({ block, isLast }) => {
  const [expanded, setExpanded] = useState(false);
  const [bashExpanded, setBashExpanded] = useState(false);
  const outputEndRef = useRef<HTMLDivElement | null>(null);
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const isAtBottomRef = useRef(true);
  const isRunning = block.status === 'running';
  const isFailed = block.status === 'failed' || block.isError;
  const isDone = block.status === 'done';

  const icon = isRunning ? '⏺' : isFailed ? '✗' : '✓';
  const iconColor = isRunning
    ? 'text-amber-500'
    : isFailed
      ? 'text-red-500'
      : 'text-emerald-500';
  const lineOpacity = isRunning || (isLast && !isDone) ? '' : isFailed ? '' : 'opacity-70';

  const detail = contentBlockSummary(block);

  // Memoize diff computation to avoid recalculating on every parent re-render
  const memoizedDiff = useMemo(() => {
    const d = block.diffData;
    if (!d || d.diff_type === 'write' || !d.old_string || !d.new_string) {
      return { text: '', stats: { added: 0, deleted: 0 } };
    }
    const text = buildInlineDiff(d.old_string, d.new_string, d.start_line);
    return { text, stats: diffStats(text) };
  }, [block.diffData]);

  const isBash = block.tool === 'Bash';
  const hasOutput = isBash && block.output && block.output.length > 0;
  const hasDiff = !!block.diffData;
  const hasWidget = hasOutput || hasDiff;

  const handleScroll = () => {
    const el = scrollContainerRef.current;
    if (!el) return;
    isAtBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 30;
  };

  useEffect(() => {
    if (isRunning && hasOutput && isAtBottomRef.current && outputEndRef.current) {
      outputEndRef.current.scrollIntoView({ behavior: 'auto', block: 'nearest' });
    }
  }, [isRunning, hasOutput, block.output?.length]);

  const showExpanded = isFailed || expanded;

  const handleClick = () => {
    if (hasWidget || block.summary) {
      setExpanded(!expanded);
    }
  };

  const bashWidget = () => {
    if (!hasOutput) return null;
    const lines = block.output!;
    const hasTimeout = block.summary?.toLowerCase().includes('timeout');

    const prefixedLine = (line: string, i: number) => (
      <div key={i} className="truncate">
        <span className="text-slate-300 dark:text-slate-600 select-none">⎿  </span>{line}
      </div>
    );

    const expandBtn = (
      <button
        onClick={(e) => { e.stopPropagation(); setBashExpanded(!bashExpanded); }}
        className="absolute top-0 right-1 text-[11px] text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 select-none z-10"
        title={bashExpanded ? 'Collapse' : 'Expand'}
      >
        {bashExpanded ? '⤡' : '⤢'}
      </button>
    );

    if (isRunning) {
      return (
        <div className="relative">
          <div
            ref={scrollContainerRef}
            onScroll={handleScroll}
            className={cn('pl-4 mt-1 overflow-y-auto custom-scrollbar', bashExpanded ? 'max-h-[70vh]' : 'max-h-80')}
          >
            <div className="font-mono text-[11px] leading-4 text-slate-600 dark:text-slate-300">
              {lines.map(prefixedLine)}
              <div ref={outputEndRef} />
            </div>
          </div>
          {expandBtn}
        </div>
      );
    }

    if (!showExpanded) {
      const previewCount = lines.length <= 3 ? lines.length : 2;
      const preview = lines.slice(0, previewCount);
      const remaining = lines.length - previewCount;
      return (
        <div className="pl-4 mt-0.5 text-[11px] text-slate-500 dark:text-slate-400 font-mono cursor-pointer" onClick={handleClick}>
          {preview.map(prefixedLine)}
          {remaining > 0 && (
            <div className="text-slate-400 dark:text-slate-500">
              <span className="text-slate-300 dark:text-slate-600 select-none">⎿  </span>… +{remaining} lines (click to expand)
            </div>
          )}
          {hasTimeout && (
            <div className="text-slate-400 dark:text-slate-500">
              <span className="text-slate-300 dark:text-slate-600 select-none">⎿  </span>(timeout)
            </div>
          )}
        </div>
      );
    }

    return (
      <div className="relative">
        <div className={cn('pl-4 mt-1 overflow-y-auto custom-scrollbar', bashExpanded ? 'max-h-[70vh]' : 'max-h-80')}>
          <div className="font-mono text-[11px] leading-4 text-slate-600 dark:text-slate-300">
            {lines.map(prefixedLine)}
            {hasTimeout && (
              <div className="text-slate-400 dark:text-slate-500">
                <span className="text-slate-300 dark:text-slate-600 select-none">⎿  </span>(timeout)
              </div>
            )}
          </div>
        </div>
        {expandBtn}
      </div>
    );
  };

  const diffWidget = () => {
    if (!hasDiff) return null;
    const d = block.diffData!;

    if (d.diff_type === 'write') {
      const lineLabel = `Wrote ${d.lines_written} line${d.lines_written === 1 ? '' : 's'}`;
      if (!showExpanded) {
        return (
          <div className="pl-4 mt-0.5 text-[11px] text-slate-400 dark:text-slate-500 font-mono cursor-pointer" onClick={handleClick}>
            <span className="text-slate-300 dark:text-slate-600 select-none">⎿  </span>
            {lineLabel}
            {d.new_content && <span className="text-slate-400 dark:text-slate-500 ml-1">(click to expand)</span>}
          </div>
        );
      }
      return (
        <div className="pl-4 mt-0.5">
          <div className="text-[11px] text-slate-400 dark:text-slate-500 font-mono mb-0.5">
            <span className="text-slate-300 dark:text-slate-600 select-none">⎿  </span>{lineLabel}
          </div>
          {d.new_content && (
            <div className="max-h-80 overflow-y-auto custom-scrollbar rounded border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-white/[0.02]">
              <pre className="text-[11px] leading-4 p-2 text-green-700 dark:text-green-400 whitespace-pre-wrap break-words">
                {d.new_content.split('\n').map((line, i) => (
                  <div key={i}>
                    <span className="text-slate-300 dark:text-slate-600 select-none inline-block w-8 text-right mr-2">{i + 1}</span>
                    <span className="text-green-600/20 dark:text-green-500/20 select-none">+</span> {line}
                  </div>
                ))}
              </pre>
            </div>
          )}
        </div>
      );
    }

    // Edit diffs: always show inline (like CC)
    if (!d.old_string || !d.new_string) return null;
    const diffText = memoizedDiff.text;
    const { added, deleted } = memoizedDiff.stats;
    const summaryParts: string[] = [];
    if (added > 0) summaryParts.push(`Added ${added} line${added !== 1 ? 's' : ''}`);
    if (deleted > 0) summaryParts.push(`removed ${deleted} line${deleted !== 1 ? 's' : ''}`);
    const summary = summaryParts.join(', ');

    return (
      <div className="pl-4 mt-0.5">
        {summary && (
          <div className="text-[11px] text-slate-400 dark:text-slate-500 font-mono mb-0.5">
            <span className="text-slate-300 dark:text-slate-600 select-none">⎿  </span>{summary}
          </div>
        )}
        <DiffView diff={diffText} />
      </div>
    );
  };

  return (
    <div className={cn('font-mono', lineOpacity)}>
      <div
        className={cn('flex items-start gap-1.5 cursor-pointer select-none')}
        onClick={handleClick}
      >
        <span className={cn('mt-px text-[11px] leading-none', iconColor, isRunning && 'animate-pulse')}>
          {icon}
        </span>
        <span className="text-[12px] min-w-0">
          <span className={cn('font-semibold', isFailed ? 'text-red-600 dark:text-red-400' : 'text-slate-700 dark:text-slate-200')}>
            {block.tool || 'Tool'}
          </span>
          {detail && (
            <span className={cn(isFailed ? 'text-red-400 dark:text-red-500' : 'text-slate-400 dark:text-slate-500')}>
              (<span>{truncateDetail(detail, 60)}</span>)
            </span>
          )}
        </span>
      </div>
      {showExpanded && !hasWidget && block.summary && (
        <div className={cn(
          'pl-4 mt-0.5 text-[11px] whitespace-pre-wrap break-words max-h-32 overflow-y-auto',
          isFailed
            ? 'text-red-500 dark:text-red-400'
            : 'text-slate-500 dark:text-slate-400'
        )}>
          {block.summary}
        </div>
      )}
      {bashWidget()}
      {diffWidget()}
    </div>
  );
});

