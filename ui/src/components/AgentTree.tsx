import React from 'react';
import { Activity, FileText, FolderOpen } from 'lucide-react';
import type { AgentTreeItem, ProjectInfo } from '../types';

type ActivityEntry = {
  path: string;
  agent: string;
  status: string;
  lastModified: number;
};

const splitPath = (path: string) => {
  const idx = path.lastIndexOf('/');
  if (idx < 0) return { folder: '.', file: path };
  return { folder: path.slice(0, idx) || '.', file: path.slice(idx + 1) };
};

const collectEntries = (
  nodeMap: Record<string, AgentTreeItem> | undefined,
  out: ActivityEntry[]
) => {
  if (!nodeMap) return;
  Object.values(nodeMap).forEach((item) => {
    if (item.type === 'file') {
      if (!item.path || !item.agent) return;
      out.push({
        path: item.path,
        agent: item.agent,
        status: item.status || 'idle',
        lastModified: Number(item.last_modified || 0),
      });
      return;
    }
    collectEntries(item.children, out);
  });
};

const activeAgentEntries = (tree: Record<string, AgentTreeItem> | undefined) => {
  const entries: ActivityEntry[] = [];
  collectEntries(tree, entries);
  const working = entries
    .filter((entry) => entry.status === 'working')
    .sort((a, b) => b.lastModified - a.lastModified);
  const byAgent = new Map<string, ActivityEntry>();
  for (const entry of working) {
    if (!byAgent.has(entry.agent)) {
      byAgent.set(entry.agent, entry);
    }
  }
  return Array.from(byAgent.entries())
    .map(([agent, entry]) => ({ agent, entry }))
    .sort((a, b) => a.agent.localeCompare(b.agent));
};

export const AgentTree: React.FC<{
  projects: ProjectInfo[];
  selectedProjectRoot: string;
  treesByProject: Record<string, Record<string, AgentTreeItem>>;
  onSelectProject: (projectRoot: string) => void;
  onSelectPath: (projectRoot: string, path: string) => void;
}> = ({
  projects,
  selectedProjectRoot,
  treesByProject,
  onSelectProject,
  onSelectPath,
}) => {
  return (
    <div className="flex-1 overflow-y-auto p-2">
      {projects.length === 0 && (
        <div className="p-4 text-xs text-slate-500 italic text-center">
          No repositories yet. Add a project to start.
        </div>
      )}
      {projects.map((project) => {
        const rows = activeAgentEntries(treesByProject[project.path]);
        const isSelected = selectedProjectRoot === project.path;
        return (
          <section
            key={project.path}
            className={`mb-2 rounded-xl border overflow-hidden ${
              isSelected
                ? 'border-blue-300 dark:border-blue-500/40 bg-blue-50/30 dark:bg-blue-500/5'
                : 'border-slate-200 dark:border-white/5 bg-white dark:bg-black/20'
            }`}
          >
            <button
              onClick={() => onSelectProject(project.path)}
              className="w-full px-3 py-2 text-left border-b border-slate-200 dark:border-white/5 hover:bg-slate-50 dark:hover:bg-white/5 transition-colors"
              title={project.path}
            >
              <div className="text-[11px] font-bold uppercase tracking-widest text-slate-500">
                {project.name}
              </div>
            </button>

            {rows.length === 0 && (
              <div className="px-3 py-2 text-[12px] italic text-slate-500">
                No active agent paths.
              </div>
            )}

            <div className="p-1.5 space-y-1">
              {rows.map(({ agent, entry }) => {
                const parts = splitPath(entry.path);
                return (
                  <button
                    key={`${project.path}:${agent}:${entry.path}`}
                    onClick={() => onSelectPath(project.path, entry.path)}
                    className="w-full text-left px-2 py-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-white/5 transition-colors"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <div className="min-w-0">
                        <div className="text-[11px] font-bold uppercase tracking-widest text-slate-500 dark:text-slate-300">
                          {agent}
                        </div>
                        <div className="text-xs font-medium text-slate-800 dark:text-slate-100 flex items-center gap-1.5 mt-0.5">
                          <FileText size={12} className="shrink-0 text-blue-500" />
                          <span className="truncate">{parts.file}</span>
                        </div>
                        <div className="text-[11px] text-slate-500 dark:text-slate-400 truncate flex items-center gap-1.5 mt-0.5">
                          <FolderOpen size={11} className="shrink-0" />
                          <span>{parts.folder}</span>
                        </div>
                      </div>
                      <span className="text-[10px] px-1.5 py-0.5 rounded-full uppercase tracking-wide shrink-0 bg-blue-500/20 text-blue-600 dark:text-blue-400 flex items-center gap-1">
                        <Activity size={9} />
                        active
                      </span>
                    </div>
                  </button>
                );
              })}
            </div>
          </section>
        );
      })}
    </div>
  );
};
