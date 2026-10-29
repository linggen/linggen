/** The reports subagents handed back, under the parent message that started
 *  them: one collapsed row each, expanding to the full report. */
import React, { useState } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import type { SubagentTreeEntry } from '../../types';
import { MarkdownContent } from './MarkdownContent';
import { firstLine } from '../../lib/paneItems.mts';

const ReportRow: React.FC<{ entry: SubagentTreeEntry }> = ({ entry }) => {
  const [expanded, setExpanded] = useState(false);
  const title = firstLine(entry.task) || entry.agentName || entry.subagentId;
  return (
    <div className="text-[12px] leading-relaxed">
      <button
        onClick={() => setExpanded((v) => !v)}
        className="inline-flex max-w-full items-center gap-1.5 px-2 py-0.5 rounded-md text-slate-500 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-white/[0.04] hover:text-slate-700 dark:hover:text-slate-200 transition-colors select-none"
        aria-expanded={expanded}
      >
        {expanded ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
        <span className="font-medium shrink-0">Subagent report</span>
        <span className="shrink-0">·</span>
        <span className="truncate" data-no-i18n>{title}</span>
      </button>
      {expanded && (
        <div className="mt-1 ml-5 border-l-2 border-slate-200 dark:border-white/10 pl-3 py-1 text-[13px] text-slate-800 dark:text-slate-200">
          <MarkdownContent text={entry.report!} />
        </div>
      )}
    </div>
  );
};

export const SubagentReports: React.FC<{ entries: SubagentTreeEntry[] }> = ({ entries }) => {
  const reported = entries.filter((entry) => entry.report);
  if (reported.length === 0) return null;
  return (
    <div className="mb-1.5 flex flex-col gap-0.5">
      {reported.map((entry) => <ReportRow key={entry.subagentId} entry={entry} />)}
    </div>
  );
};
