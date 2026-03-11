import React, { useState, useRef, useEffect, useMemo, useCallback } from 'react';
import { createPortal } from 'react-dom';
import {
  Activity,
  ChevronDown,
  ChevronRight,
  FileText,
  FolderOpen,
  FolderPlus,
  MoreHorizontal,
  Plus,
} from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentTreeItem, ProjectInfo, SessionInfo } from '../types';

// ---- Helpers ----------------------------------------------------------------

const relativeTime = (epochSecs: number): string => {
  const diff = Date.now() / 1000 - epochSecs;
  if (diff < 60) return 'just now';
  if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  if (diff < 172800) return 'yesterday';
  return `${Math.floor(diff / 86400)}d ago`;
};

type ActivityEntry = {
  path: string;
  agent: string;
  status: string;
  lastModified: number;
};

const collectEntries = (
  nodeMap: Record<string, AgentTreeItem> | undefined,
  out: ActivityEntry[],
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
    .filter((e) => e.status === 'working')
    .sort((a, b) => b.lastModified - a.lastModified);
  const byAgent = new Map<string, ActivityEntry>();
  for (const entry of working) {
    if (!byAgent.has(entry.agent)) byAgent.set(entry.agent, entry);
  }
  return Array.from(byAgent.entries())
    .map(([agent, entry]) => ({ agent, entry }))
    .sort((a, b) => a.agent.localeCompare(b.agent));
};

const splitPath = (path: string) => {
  const idx = path.lastIndexOf('/');
  if (idx < 0) return { folder: '.', file: path };
  return { folder: path.slice(0, idx) || '.', file: path.slice(idx + 1) };
};

const truncatePath = (path: string, maxLen = 32) => {
  if (path.length <= maxLen) return path;
  return '...' + path.slice(-(maxLen - 3));
};

// ---- Dropdown portal --------------------------------------------------------

const DropdownMenu: React.FC<{
  anchorRef: React.RefObject<HTMLButtonElement | null>;
  open: boolean;
  onClose: () => void;
  children: React.ReactNode;
}> = ({ anchorRef, open, onClose, children }) => {
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  useEffect(() => {
    if (!open || !anchorRef.current) {
      setPos(null);
      return;
    }
    const rect = anchorRef.current.getBoundingClientRect();
    setPos({ top: rect.bottom + 4, left: rect.right - 140 });
  }, [open, anchorRef]);

  useEffect(() => {
    if (!open) return;
    const handler = () => onClose();
    // Delay so the triggering click doesn't immediately close
    const id = requestAnimationFrame(() => {
      window.addEventListener('click', handler);
    });
    return () => {
      cancelAnimationFrame(id);
      window.removeEventListener('click', handler);
    };
  }, [open, onClose]);

  if (!open || !pos) return null;

  return createPortal(
    <div
      style={{ position: 'fixed', top: pos.top, left: pos.left, zIndex: 9999 }}
      className="bg-white dark:bg-[#1a1a1a] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl py-1 min-w-[140px]"
      onClick={(e) => e.stopPropagation()}
    >
      {children}
    </div>,
    document.body,
  );
};

// ---- Component Props --------------------------------------------------------

export interface SessionNavProps {
  projects: ProjectInfo[];
  selectedProjectRoot: string;
  setSelectedProjectRoot: (root: string) => void;

  sessions: SessionInfo[];
  activeSessionId: string | null;
  setActiveSessionId: (id: string | null) => void;
  createSession: () => void;
  removeSession: (id: string) => void;
  renameSession: (id: string, title: string) => void;

  sessionCountsByProject: Record<string, number>;

  treesByProject: Record<string, Record<string, AgentTreeItem>>;
  onSelectPath: (projectRoot: string, path: string) => void;

  pickFolder: () => void;
  removeProject: (path: string) => void;
}

// ---- SessionNav Component ---------------------------------------------------

export const SessionNav: React.FC<SessionNavProps> = ({
  projects,
  selectedProjectRoot,
  setSelectedProjectRoot,
  sessions,
  activeSessionId,
  setActiveSessionId,
  createSession,
  removeSession,
  renameSession,
  sessionCountsByProject,
  treesByProject,
  onSelectPath,
  pickFolder,
  removeProject,
}) => {
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(() =>
    selectedProjectRoot ? new Set([selectedProjectRoot]) : new Set(),
  );
  const [renamingSessionId, setRenamingSessionId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [menuSessionId, setMenuSessionId] = useState<string | null>(null);
  const [menuProjectPath, setMenuProjectPath] = useState<string | null>(null);
  const [liveActivityOpen, setLiveActivityOpen] = useState(true);
  const renameInputRef = useRef<HTMLInputElement>(null);
  const projectMenuRef = useRef<HTMLButtonElement>(null);
  const sessionMenuRef = useRef<HTMLButtonElement>(null);

  // Auto-expand selected project
  useEffect(() => {
    if (selectedProjectRoot) {
      setExpandedProjects((prev) => {
        if (prev.has(selectedProjectRoot)) return prev;
        const next = new Set(prev);
        next.add(selectedProjectRoot);
        return next;
      });
    }
  }, [selectedProjectRoot]);

  // Focus rename input when editing
  useEffect(() => {
    if (renamingSessionId && renameInputRef.current) {
      renameInputRef.current.focus();
      renameInputRef.current.select();
    }
  }, [renamingSessionId]);

  const closeMenus = useCallback(() => {
    setMenuSessionId(null);
    setMenuProjectPath(null);
  }, []);

  const toggleProject = (path: string) => {
    setExpandedProjects((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
    setSelectedProjectRoot(path);
  };

  const startRename = (session: SessionInfo) => {
    setRenamingSessionId(session.id);
    setRenameValue(session.title);
    setMenuSessionId(null);
  };

  const commitRename = () => {
    if (renamingSessionId && renameValue.trim()) {
      renameSession(renamingSessionId, renameValue.trim());
    }
    setRenamingSessionId(null);
  };

  // Auto-expand the selected project so sessions are visible
  useEffect(() => {
    if (selectedProjectRoot) {
      setExpandedProjects((prev) => {
        if (prev.has(selectedProjectRoot)) return prev;
        const next = new Set(prev);
        next.add(selectedProjectRoot);
        return next;
      });
    }
  }, [selectedProjectRoot]);

  // Compute live activity entries for selected project
  const liveEntries = useMemo(() => {
    if (!selectedProjectRoot) return [];
    return activeAgentEntries(treesByProject[selectedProjectRoot]);
  }, [selectedProjectRoot, treesByProject]);

  return (
    <div className="flex-1 flex flex-col min-h-0">
      {/* Header: New Chat + Add Project */}
      <div className="px-3 py-2 border-b border-slate-200 dark:border-white/5 flex items-center justify-between">
        <span className="text-[10px] font-bold uppercase tracking-wider text-slate-400">Sessions</span>
        <div className="flex items-center gap-1">
          <button
            onClick={createSession}
            disabled={!selectedProjectRoot}
            className="p-1.5 rounded-lg bg-blue-600 text-white hover:bg-blue-700 transition-colors disabled:opacity-40"
            title="New Chat"
          >
            <Plus size={14} strokeWidth={2.5} />
          </button>
          <button
            onClick={pickFolder}
            className="p-1.5 hover:bg-slate-100 dark:hover:bg-white/5 rounded-lg text-slate-500 transition-colors"
            title="Add Project"
          >
            <FolderPlus size={14} />
          </button>
        </div>
      </div>

      {/* Project tree (scrollable) */}
      <div className="flex-1 overflow-y-auto p-2 space-y-1">
        {projects.length === 0 && (
          <div className="p-4 text-xs text-slate-500 italic text-center">
            No projects yet. Add one to start.
          </div>
        )}

        {projects.map((project) => {
          const isSelected = selectedProjectRoot === project.path;
          const isExpanded = expandedProjects.has(project.path);
          const projectSessions = isExpanded
            ? (isSelected ? sessions : [])
            : [];
          const sessionCount = sessionCountsByProject[project.path] ?? 0;

          return (
            <div key={project.path} className="rounded-lg">
              {/* Project header */}
              <div className="relative group">
                <button
                  onClick={() => toggleProject(project.path)}
                  className={cn(
                    'w-full text-left px-2.5 py-2 rounded-lg transition-colors',
                    isSelected
                      ? 'bg-blue-50 dark:bg-blue-500/10'
                      : 'hover:bg-slate-50 dark:hover:bg-white/5',
                  )}
                >
                  <div className="flex items-center gap-1.5">
                    {isExpanded ? (
                      <ChevronDown size={13} className="text-slate-400 shrink-0" />
                    ) : (
                      <ChevronRight size={13} className="text-slate-400 shrink-0" />
                    )}
                    <span className="text-[11px] font-bold text-slate-800 dark:text-slate-200 truncate">
                      {project.name}
                    </span>
                  </div>
                  <div className="ml-5 text-[10px] text-slate-400 truncate mt-0.5">
                    {truncatePath(project.path)}
                  </div>
                  {!isExpanded && sessionCount > 0 && (
                    <div className="ml-5 text-[10px] text-slate-400 mt-0.5">
                      ({sessionCount} session{sessionCount !== 1 ? 's' : ''})
                    </div>
                  )}
                </button>

                {/* Project context menu trigger */}
                <button
                  ref={menuProjectPath === project.path ? projectMenuRef : undefined}
                  onClick={(e) => {
                    e.stopPropagation();
                    setMenuSessionId(null);
                    setMenuProjectPath(menuProjectPath === project.path ? null : project.path);
                  }}
                  className="absolute right-2 top-2 p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-slate-200 dark:hover:bg-white/10 text-slate-400 transition-all"
                >
                  <MoreHorizontal size={13} />
                </button>
                <DropdownMenu anchorRef={projectMenuRef} open={menuProjectPath === project.path} onClose={closeMenus}>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      setMenuProjectPath(null);
                      removeProject(project.path);
                    }}
                    className="w-full text-left px-3 py-1.5 text-[11px] text-red-500 hover:bg-red-50 dark:hover:bg-red-500/10"
                  >
                    Remove Project
                  </button>
                </DropdownMenu>
              </div>

              {/* Sessions list */}
              {isExpanded && (
                <div className="ml-3 mt-0.5 space-y-0.5">
                  {/* Sessions */}
                  {projectSessions.map((session) => {
                    const isActive = isSelected && activeSessionId === session.id;
                    const isRenaming = renamingSessionId === session.id;

                    return (
                      <div key={session.id} className="relative group/session">
                        <button
                          onClick={() => {
                            setSelectedProjectRoot(project.path);
                            setActiveSessionId(session.id);
                          }}
                          onDoubleClick={() => startRename(session)}
                          className={cn(
                            'w-full text-left px-2.5 py-2 rounded-lg transition-colors text-[11px]',
                            isActive
                              ? 'bg-blue-100/80 dark:bg-blue-500/15 border-l-2 border-blue-500'
                              : 'hover:bg-slate-50 dark:hover:bg-white/5',
                          )}
                        >
                          {isRenaming ? (
                            <input
                              ref={renameInputRef}
                              value={renameValue}
                              onChange={(e) => setRenameValue(e.target.value)}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') commitRename();
                                if (e.key === 'Escape') setRenamingSessionId(null);
                              }}
                              onBlur={commitRename}
                              onClick={(e) => e.stopPropagation()}
                              className="w-full bg-white dark:bg-black/30 border border-blue-400 rounded px-1.5 py-0.5 text-[11px] outline-none"
                            />
                          ) : (
                            <>
                              <div className="font-medium text-slate-800 dark:text-slate-200 truncate pr-6">
                                {session.title}
                              </div>
                              <div className="text-[10px] text-slate-400 mt-0.5">
                                {relativeTime(session.created_at)}
                              </div>
                            </>
                          )}
                        </button>

                        {/* Session context menu trigger */}
                        {!isRenaming && (
                          <button
                            ref={menuSessionId === session.id ? sessionMenuRef : undefined}
                            onClick={(e) => {
                              e.stopPropagation();
                              setMenuProjectPath(null);
                              setMenuSessionId(menuSessionId === session.id ? null : session.id);
                            }}
                            className="absolute right-1.5 top-2 p-1 rounded opacity-0 group-hover/session:opacity-100 hover:bg-slate-200 dark:hover:bg-white/10 text-slate-400 transition-all"
                          >
                            <MoreHorizontal size={12} />
                          </button>
                        )}
                        <DropdownMenu anchorRef={sessionMenuRef} open={menuSessionId === session.id} onClose={closeMenus}>
                          <button
                            onClick={(e) => {
                              e.stopPropagation();
                              startRename(session);
                            }}
                            className="w-full text-left px-3 py-1.5 text-[11px] hover:bg-slate-50 dark:hover:bg-white/5"
                          >
                            Rename
                          </button>
                          <button
                            onClick={(e) => {
                              e.stopPropagation();
                              setMenuSessionId(null);
                              removeSession(session.id);
                            }}
                            className="w-full text-left px-3 py-1.5 text-[11px] text-red-500 hover:bg-red-50 dark:hover:bg-red-500/10"
                          >
                            Delete
                          </button>
                        </DropdownMenu>
                      </div>
                    );
                  })}

                </div>
              )}
            </div>
          );
        })}
      </div>

      {/* Live Activity Footer */}
      {liveEntries.length > 0 && (
        <div className="border-t border-slate-200 dark:border-white/5">
          <button
            onClick={() => setLiveActivityOpen(!liveActivityOpen)}
            className="w-full px-3 py-2 flex items-center justify-between text-[10px] font-bold uppercase tracking-wider text-slate-500 hover:bg-slate-50 dark:hover:bg-white/5"
          >
            <div className="flex items-center gap-1.5">
              {liveActivityOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
              <Activity size={12} />
              Live Activity
            </div>
            <span className="text-[9px] bg-blue-500/15 text-blue-600 dark:text-blue-400 px-1.5 py-0.5 rounded-full">
              {liveEntries.length}
            </span>
          </button>
          {liveActivityOpen && (
            <div className="px-3 pb-2 space-y-1">
              {liveEntries.map(({ agent, entry }) => {
                const parts = splitPath(entry.path);
                return (
                  <button
                    key={`${agent}:${entry.path}`}
                    onClick={() => onSelectPath(selectedProjectRoot, entry.path)}
                    className="w-full text-left px-2 py-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-white/5 transition-colors"
                  >
                    <div className="flex items-center gap-2 text-[10px]">
                      <span className="font-bold text-slate-500 uppercase">{agent}</span>
                      <span className="text-slate-300 dark:text-slate-600">&rarr;</span>
                      <span className="text-slate-800 dark:text-slate-100 flex items-center gap-1 truncate">
                        <FileText size={10} className="shrink-0 text-blue-500" />
                        {parts.file}
                      </span>
                      <span className="text-[9px] text-blue-500 ml-auto shrink-0">active</span>
                    </div>
                    <div className="text-[9px] text-slate-400 mt-0.5 flex items-center gap-1 ml-0.5">
                      <FolderOpen size={9} className="shrink-0" />
                      <span className="truncate">{parts.folder}</span>
                    </div>
                  </button>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
};
