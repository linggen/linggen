import React, { useEffect, useRef, useState } from 'react';
import { cn } from '../../lib/cn';
import DiffView from '../DiffView';
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
    <div className="mt-1.5 flex items-center gap-1.5 text-[10px] font-mono text-slate-400 dark:text-slate-500 select-none">
      <span className="w-1 h-1 rounded-full bg-slate-300 dark:bg-slate-600" />
      {parts.join(' · ')}
    </div>
  );
};

/** Render a single ContentBlock as a compact tool line with optional Bash output / diff widget. */
export const ContentBlockView: React.FC<{
  block: ContentBlock;
  isLast: boolean;
}> = ({ block, isLast }) => {
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
        className="absolute top-0 right-1 text-[10px] text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 select-none z-10"
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
            <div className="font-mono text-[10px] leading-4 text-slate-600 dark:text-slate-300">
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
        <div className="pl-4 mt-0.5 text-[10px] text-slate-500 dark:text-slate-400 font-mono cursor-pointer" onClick={handleClick}>
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
          <div className="font-mono text-[10px] leading-4 text-slate-600 dark:text-slate-300">
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
      if (!showExpanded) {
        return (
          <div className="pl-4 mt-0.5 text-[10px] text-slate-400 dark:text-slate-500 font-mono cursor-pointer" onClick={handleClick}>
            Wrote {d.lines_written} line{d.lines_written === 1 ? '' : 's'}
          </div>
        );
      }
      return (
        <div className="pl-4 mt-0.5 text-[10px] text-slate-500 dark:text-slate-400 font-mono">
          Wrote {d.lines_written} line{d.lines_written === 1 ? '' : 's'} to {d.path}
        </div>
      );
    }

    if (!d.old_string || !d.new_string) return null;
    const stats = (() => {
      const oldLines = d.old_string!.split('\n').length;
      const newLines = d.new_string!.split('\n').length;
      const added = Math.max(0, newLines - oldLines) || newLines;
      const removed = Math.max(0, oldLines - newLines) || oldLines;
      return { added, removed };
    })();

    if (!showExpanded) {
      return (
        <div className="pl-4 mt-0.5 text-[10px] text-slate-400 dark:text-slate-500 font-mono cursor-pointer" onClick={handleClick}>
          <span className="text-green-600 dark:text-green-400">+{stats.added}</span>
          {' / '}
          <span className="text-red-600 dark:text-red-400">-{stats.removed}</span>
          {' lines (click to expand)'}
        </div>
      );
    }

    const diffText = buildInlineDiff(d.old_string!, d.new_string!, d.start_line);
    return (
      <div className="pl-4 mt-1">
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
        <span className={cn('mt-px text-[10px] leading-none', iconColor, isRunning && 'animate-pulse')}>
          {icon}
        </span>
        <span className="text-[11px] min-w-0">
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
          'pl-4 mt-0.5 text-[10px] whitespace-pre-wrap break-words max-h-32 overflow-y-auto',
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
};

/** Renders msg.content[] tool_use blocks as an action widget with collapsible header. */
export const ContentBlockList: React.FC<{
  blocks: ContentBlock[];
  isGenerating: boolean;
}> = ({ blocks, isGenerating }) => {
  const toolBlocks = blocks.filter(b => b.type === 'tool_use');
  const hasRunning = toolBlocks.some(b => b.status === 'running');
  const allDone = !isGenerating && !hasRunning;

  const [collapsed, setCollapsed] = useState(false);
  const prevAllDone = useRef(allDone);
  useEffect(() => {
    if (allDone && !prevAllDone.current && toolBlocks.length > 3) {
      setCollapsed(true);
    }
    prevAllDone.current = allDone;
  }, [allDone, toolBlocks.length]);

  const listRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!collapsed && listRef.current && hasRunning) {
      const el = listRef.current;
      el.scrollTop = el.scrollHeight;
    }
  }, [blocks.length, hasRunning, collapsed]);

  if (toolBlocks.length === 0) return null;

  const headerIcon = allDone ? '✓' : '⚡';
  const headerColor = allDone
    ? 'text-emerald-600 dark:text-emerald-400'
    : 'text-amber-600 dark:text-amber-400';
  const headerText = `${toolBlocks.length} tool${toolBlocks.length === 1 ? '' : 's'} called`;

  if (collapsed) {
    return (
      <div
        className="mb-1.5 text-[11px] cursor-pointer select-none flex items-center gap-1.5 font-mono"
        onClick={() => setCollapsed(false)}
      >
        <span className="text-[9px] text-slate-400 dark:text-slate-500">▶</span>
        <span className={cn('font-medium', headerColor)}>{headerIcon} {headerText}</span>
      </div>
    );
  }

  return (
    <div className="mb-1.5">
      <div
        className={cn(
          'flex items-center gap-1.5 text-[11px] font-mono select-none',
          allDone && 'cursor-pointer'
        )}
        onClick={allDone ? () => setCollapsed(true) : undefined}
      >
        {allDone && <span className="text-[9px] text-slate-400 dark:text-slate-500">▼</span>}
        <span className={cn('font-medium', headerColor)}>{headerIcon} {headerText}</span>
      </div>
      <div
        ref={listRef}
        className={cn(
          "mt-0.5 pl-1 space-y-0.5 overflow-y-auto custom-scrollbar",
          allDone ? 'max-h-[180px]' : 'max-h-[480px]'
        )}
      >
        {toolBlocks.map((block, idx) => (
          <ContentBlockView
            key={block.id || `cb-${idx}`}
            block={block}
            isLast={idx === toolBlocks.length - 1}
          />
        ))}
      </div>
    </div>
  );
};
