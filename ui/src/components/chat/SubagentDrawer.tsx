import React from 'react';
import { X } from 'lucide-react';
import { cn } from '../../lib/cn';
import type {
  AgentRunInfo,
  AgentRunContextResponse,
  ChatMessage,
  SubagentInfo,
} from '../../types';
import type { TimelineEvent } from './types';
import { formatRunLabel, formatTs } from './utils/timeline';
import { MarkdownContent } from './MarkdownContent';
import { visibleMessageText, statusBadgeClass } from './MessageHelpers';

export const SubagentDrawer: React.FC<{
  selectedSubagent: SubagentInfo;
  selectedSubagentKey: string;
  selectedSubagentRunId?: string;
  selectedSubagentRunningRunId?: string;
  selectedSubagentRunOptions: AgentRunInfo[];
  selectedSubagentPinned: boolean;
  selectedSubagentContext?: AgentRunContextResponse;
  selectedSubagentContextLoading: boolean;
  selectedSubagentContextError?: string;
  selectedSubagentChildrenLoading: boolean;
  selectedSubagentChildrenError?: string;
  selectedSubagentTimeline: TimelineEvent[];
  filteredSubagentMessages: ChatMessage[];
  subagentMessageFilter: string;
  setSubagentMessageFilter: (value: string) => void;
  cancellingRunIds?: Record<string, boolean>;
  onCancelRun?: (runId: string) => void | Promise<void>;
  onClose: () => void;
  setSelectedSubagentRunById: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  setPinnedSubagentRunById: React.Dispatch<React.SetStateAction<Record<string, boolean>>>;
}> = ({
  selectedSubagent,
  selectedSubagentKey,
  selectedSubagentRunId,
  selectedSubagentRunningRunId,
  selectedSubagentRunOptions,
  selectedSubagentPinned,
  selectedSubagentContext,
  selectedSubagentContextLoading,
  selectedSubagentContextError,
  selectedSubagentChildrenLoading,
  selectedSubagentChildrenError,
  selectedSubagentTimeline,
  filteredSubagentMessages,
  subagentMessageFilter,
  setSubagentMessageFilter,
  cancellingRunIds,
  onCancelRun,
  onClose,
  setSelectedSubagentRunById,
  setPinnedSubagentRunById,
}) => (
  <div className="absolute inset-y-0 right-0 w-[min(26rem,95%)] bg-white dark:bg-[#0f0f0f] border-l border-slate-200 dark:border-white/10 shadow-2xl z-[65] flex flex-col">
    <div className="px-4 py-3 border-b border-slate-200 dark:border-white/10 flex items-start justify-between gap-3">
      <div>
        <div className="text-xs font-bold uppercase tracking-wider text-slate-500">Subagent Context</div>
        <div className="mt-1 text-sm font-semibold text-slate-900 dark:text-slate-100">{selectedSubagent.id}</div>
        <div className="mt-1 text-[11px] text-slate-500 dark:text-slate-400">
          {selectedSubagent.folder}/{selectedSubagent.file}
        </div>
      </div>
      <div className="flex items-center gap-2">
        <span className={cn('text-[11px] px-2 py-1 rounded-full uppercase tracking-wide', statusBadgeClass(selectedSubagent.status))}>
          {selectedSubagent.status}
        </span>
        {selectedSubagentRunningRunId && onCancelRun && (
          <button
            onClick={() => onCancelRun(selectedSubagentRunningRunId)}
            disabled={!!cancellingRunIds?.[selectedSubagentRunningRunId]}
            className={cn(
              'px-2 py-1 rounded-lg text-[11px] font-semibold border transition-colors',
              cancellingRunIds?.[selectedSubagentRunningRunId]
                ? 'bg-slate-100 text-slate-400 border-slate-200 cursor-not-allowed'
                : 'bg-red-50 text-red-600 border-red-200 hover:bg-red-100'
            )}
            title={selectedSubagentRunningRunId}
          >
            {cancellingRunIds?.[selectedSubagentRunningRunId] ? 'Cancelling...' : 'Cancel Run'}
          </button>
        )}
        <button
          onClick={onClose}
          className="p-1.5 rounded-lg hover:bg-slate-100 dark:hover:bg-white/10 text-slate-500"
          title="Close"
        >
          <X size={14} />
        </button>
      </div>
    </div>

    <div className="p-4 border-b border-slate-200 dark:border-white/10">
      <div className="text-[11px] uppercase tracking-widest text-slate-500 mb-2">Active Paths</div>
      <div className="space-y-1 max-h-28 overflow-auto custom-scrollbar">
        {selectedSubagent.paths.map((path) => (
          <div key={path} className="text-[12px] font-mono text-slate-600 dark:text-slate-300 truncate">
            {path}
          </div>
        ))}
      </div>
    </div>

    <div className="flex-1 overflow-auto p-4 space-y-3 custom-scrollbar">
      <div className="text-[11px] uppercase tracking-widest text-slate-500">Messages</div>
      {selectedSubagentRunId && (
        <div className="rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-white/5 px-2.5 py-2 text-[11px] text-slate-500 dark:text-slate-400">
          <div className="font-mono text-slate-600 dark:text-slate-300 break-all">{selectedSubagentRunId}</div>
          {selectedSubagentRunOptions.length > 1 && (
            <div className="mt-1">
              <select
                value={selectedSubagentRunId}
                onChange={(e) => {
                  const runId = e.target.value;
                  if (!selectedSubagentKey) return;
                  setSelectedSubagentRunById((prev) => ({ ...prev, [selectedSubagentKey]: runId }));
                  setPinnedSubagentRunById((prev) => ({ ...prev, [selectedSubagentKey]: true }));
                }}
                className="w-full text-[11px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none"
                title="Select subagent run context"
              >
                {selectedSubagentRunOptions.map((run) => (
                  <option key={run.run_id} value={run.run_id}>
                    {formatRunLabel(run)}
                  </option>
                ))}
              </select>
            </div>
          )}
          {selectedSubagentKey && selectedSubagentRunId && (
            <div className="mt-1">
              <button
                onClick={() => {
                  if (selectedSubagentPinned) {
                    setPinnedSubagentRunById((prev) => {
                      const next = { ...prev };
                      delete next[selectedSubagentKey];
                      return next;
                    });
                    setSelectedSubagentRunById((prev) => {
                      const next = { ...prev };
                      delete next[selectedSubagentKey];
                      return next;
                    });
                  } else {
                    setSelectedSubagentRunById((prev) => ({ ...prev, [selectedSubagentKey]: selectedSubagentRunId }));
                    setPinnedSubagentRunById((prev) => ({ ...prev, [selectedSubagentKey]: true }));
                  }
                }}
                className={cn(
                  'px-2 py-1 rounded border text-[11px] font-semibold',
                  selectedSubagentPinned
                    ? 'bg-slate-100 text-slate-600 border-slate-300'
                    : 'bg-blue-50 text-blue-600 border-blue-200'
                )}
              >
                {selectedSubagentPinned ? 'Unpin' : 'Pin'}
              </button>
            </div>
          )}
          {selectedSubagentContext?.summary && (
            <div className="mt-1">
              messages: {selectedSubagentContext.summary.message_count} • user: {selectedSubagentContext.summary.user_messages} • agent: {selectedSubagentContext.summary.agent_messages} • system: {selectedSubagentContext.summary.system_messages}
            </div>
          )}
          {selectedSubagentContextLoading && <div className="mt-1 text-blue-500">Loading context...</div>}
          {selectedSubagentContextError && <div className="mt-1 text-red-500">Context error: {selectedSubagentContextError}</div>}
          {selectedSubagentChildrenLoading && <div className="mt-1 text-blue-500">Loading child runs...</div>}
          {selectedSubagentChildrenError && <div className="mt-1 text-red-500">Children error: {selectedSubagentChildrenError}</div>}
        </div>
      )}
      <div className="rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50/70 dark:bg-white/[0.03] px-2.5 py-2 space-y-2">
        <div className="flex items-center justify-between gap-2">
          <div className="text-[11px] uppercase tracking-widest text-slate-500">Context Tools</div>
          <input
            value={subagentMessageFilter}
            onChange={(e) => setSubagentMessageFilter(e.target.value)}
            placeholder="Filter messages"
            className="w-40 text-[12px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none"
          />
        </div>
        {selectedSubagentTimeline.length > 0 && (
          <details>
            <summary className="cursor-pointer text-[12px] font-semibold text-slate-600 dark:text-slate-300">
              Timeline ({selectedSubagentTimeline.length})
            </summary>
            <div className="mt-1.5 space-y-1.5 max-h-28 overflow-auto custom-scrollbar">
              {selectedSubagentTimeline.map((evt, idx) => (
                <div key={`${evt.ts}-${evt.label}-${idx}`} className="text-[12px] text-slate-600 dark:text-slate-300">
                  <span className="font-mono text-[11px] text-slate-500 mr-2">{formatTs(evt.ts)}</span>
                  <span className="font-semibold">{evt.label}</span>
                  {evt.detail && <span className="text-slate-500"> • {evt.detail}</span>}
                </div>
              ))}
            </div>
          </details>
        )}
      </div>
      {filteredSubagentMessages.length === 0 && (
        <div className="text-xs italic text-slate-500">No context messages captured for this subagent yet.</div>
      )}
      {filteredSubagentMessages.slice(-20).map((msg, idx) => (
        <div key={`${msg.timestamp}-${idx}-${msg.from || msg.role}`} className="rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-white/5 p-2.5">
          <div className="text-[10px] uppercase tracking-wider text-slate-500 mb-1">
            {(msg.from || msg.role).toUpperCase()} {msg.to ? `\u2192 ${msg.to.toUpperCase()}` : ''}
          </div>
          <div className="text-[13px] text-slate-700 dark:text-slate-200 break-words">
            <MarkdownContent text={visibleMessageText(msg)} />
          </div>
        </div>
      ))}
    </div>
  </div>
);
