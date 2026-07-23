import React from 'react';

const sectionCls = 'bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm p-5';

interface ToolDef {
  name: string;
  description: string;
  note?: string; // small right-aligned hint, e.g. "Requires sign-in"
}

const TOOLS: ToolDef[] = [
  { name: 'Read', description: 'Read files' },
  { name: 'Write', description: 'Write files' },
  { name: 'Edit', description: 'Edit files' },
  { name: 'Bash', description: 'Run shell commands' },
  { name: 'Glob', description: 'Find files by pattern' },
  { name: 'Grep', description: 'Search file contents' },
  { name: 'WebSearch', description: 'Search the web', note: 'Requires sign-in' },
  { name: 'WebFetch', description: 'Fetch web page content' },
  { name: 'Skill', description: 'Run skills' },
  { name: 'AskUser', description: 'Ask user questions' },
  { name: 'Task', description: 'Delegate to subagent' },
];

export const ToolsTab: React.FC = () => {
  return (
    <div className="space-y-6">
      <section className={sectionCls}>
        <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-1">
          Built-in Tools
        </h2>
        <p className="text-[11px] text-slate-400 mb-4">
          Tools available to agents during execution.
        </p>

        <div className="space-y-1">
          {TOOLS.map((tool) => (
            <div key={tool.name} className="flex items-center gap-3 px-3 py-2.5 bg-slate-50 dark:bg-white/[0.02] rounded-lg border border-slate-100 dark:border-white/5">
              <span className="text-xs font-mono font-semibold text-slate-700 dark:text-slate-200 w-40 shrink-0">
                {tool.name}
              </span>
              <span className="text-[12px] text-slate-500 dark:text-slate-400 flex-1">
                {tool.description}
              </span>
              {tool.note && (
                <span className="shrink-0 text-[10px] font-medium text-amber-600 dark:text-amber-400/90 bg-amber-500/10 rounded-md px-2 py-0.5">
                  {tool.note}
                </span>
              )}
            </div>
          ))}
        </div>

        <p className="mt-3 text-[11px] text-slate-400">
          Web search runs on your Linggen account — sign in to enable it. No API key needed.
        </p>
      </section>
    </div>
  );
};
