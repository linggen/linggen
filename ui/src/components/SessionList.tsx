/**
 * Unified left panel — sessions, workspace, and missions in one view.
 *
 * Three sections:
 * 1. SESSIONS — chronological list with filters and time grouping
 * 2. WORKSPACE — project management (add/remove)
 * 3. MISSIONS — mission list with inline trigger
 */
import React, { useState, useMemo, useRef, useEffect, useCallback } from 'react';
import {
  MessageSquare, Bot, Sparkles, Plus, Search, X, Trash2,
  FolderOpen, Play, Pause, ChevronDown, ChevronRight, Settings, FolderPlus,
} from 'lucide-react';
import { cn } from '../lib/cn';
import type { SessionInfo, CronMission } from '../types';
import { useProjectStore } from '../stores/projectStore';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

type CreatorFilter = 'all' | 'user' | 'mission' | 'skill';

const creatorIcon = (creator?: string) => {
  switch (creator) {
    case 'mission': return <Bot size={13} className="text-amber-500 shrink-0" />;
    case 'skill':   return <Sparkles size={13} className="text-purple-500 shrink-0" />;
    default:        return <MessageSquare size={13} className="text-blue-500 shrink-0" />;
  }
};

const creatorBadge = (creator?: string) => {
  switch (creator) {
    case 'mission': return 'bg-amber-500/10 text-amber-600 dark:text-amber-400';
    case 'skill':   return 'bg-purple-500/10 text-purple-600 dark:text-purple-400';
    default:        return 'bg-blue-500/10 text-blue-600 dark:text-blue-400';
  }
};

const relativeTime = (epochSecs: number): string => {
  const diff = Date.now() / 1000 - epochSecs;
  if (diff < 60) return 'now';
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  if (diff < 172800) return '1d';
  return `${Math.floor(diff / 86400)}d`;
};

type TimeGroup = 'Today' | 'Yesterday' | 'This Week' | 'Older';

const timeGroup = (epochSecs: number): TimeGroup => {
  const now = new Date();
  const d = new Date(epochSecs * 1000);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const yesterday = new Date(today.getTime() - 86400000);
  const weekAgo = new Date(today.getTime() - 7 * 86400000);
  if (d >= today) return 'Today';
  if (d >= yesterday) return 'Yesterday';
  if (d >= weekAgo) return 'This Week';
  return 'Older';
};

const groupOrder: Record<TimeGroup, number> = { Today: 0, Yesterday: 1, 'This Week': 2, Older: 3 };

// ---------------------------------------------------------------------------
// New Chat Dialog
// ---------------------------------------------------------------------------

const HOME_PATH = '~';

const NewChatDialog: React.FC<{
  projects: { path: string }[];
  defaultProject: string;
  onClose: () => void;
  onCreate: (projectRoot: string) => void;
}> = ({ projects, defaultProject, onClose, onCreate }) => {
  const [selected, setSelected] = useState(defaultProject || projects[0]?.path || HOME_PATH);

  return (
    <div className="absolute left-0 right-0 top-0 z-20 mx-2 mt-1 bg-white dark:bg-[#1a1a1a] border border-slate-200 dark:border-white/10 rounded-lg shadow-lg p-3 animate-slide-in">
      <div className="flex items-center justify-between mb-2">
        <span className="text-xs font-semibold text-slate-700 dark:text-slate-300">New Chat</span>
        <button onClick={onClose} className="p-0.5 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400">
          <X size={12} />
        </button>
      </div>
      <label className="text-[10px] text-slate-500 font-medium">Project</label>
      <select
        value={selected}
        onChange={(e) => setSelected(e.target.value)}
        className="w-full mt-0.5 mb-2 text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-2 py-1.5 outline-none focus:ring-1 focus:ring-blue-400 text-slate-700 dark:text-slate-300"
      >
        <option value={HOME_PATH}>Home (~)</option>
        {projects.map((p) => {
          const name = p.path.split('/').pop() || p.path;
          return <option key={p.path} value={p.path}>{name}</option>;
        })}
      </select>
      <div className="flex justify-end gap-1.5">
        <button onClick={onClose} className="px-2.5 py-1 text-[11px] rounded border border-slate-200 dark:border-white/10 text-slate-500 hover:bg-slate-50 dark:hover:bg-white/5">
          Cancel
        </button>
        <button
          onClick={() => { onCreate(selected); onClose(); }}
          disabled={!selected}
          className="px-2.5 py-1 text-[11px] rounded bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-40 font-medium"
        >
          Create
        </button>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Section Header (collapsible)
// ---------------------------------------------------------------------------

const SectionHeader: React.FC<{
  title: string;
  open: boolean;
  onToggle: () => void;
  action?: React.ReactNode;
}> = ({ title, open, onToggle, action }) => (
  <div className="flex items-center justify-between px-3 py-1.5 border-t border-slate-200 dark:border-white/5 bg-slate-50/50 dark:bg-white/[0.02] cursor-pointer select-none"
    onClick={onToggle}>
    <div className="flex items-center gap-1">
      {open ? <ChevronDown size={10} className="text-slate-400" /> : <ChevronRight size={10} className="text-slate-400" />}
      <span className="text-[9px] font-bold uppercase tracking-widest text-slate-400">{title}</span>
    </div>
    {action && <div onClick={(e) => e.stopPropagation()}>{action}</div>}
  </div>
);

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export const SessionList: React.FC<{
  activeSessionId: string | null;
  onSelectSession: (session: SessionInfo) => void;
  onCreateSession: () => void;
  onDeleteSession?: (id: string) => void;
  onOpenSettings?: (tab?: string) => void;
}> = ({ activeSessionId, onSelectSession, onCreateSession, onDeleteSession, onOpenSettings }) => {
  const allSessions = useProjectStore((s) => s.allSessions);
  const projects = useProjectStore((s) => s.projects);
  const selectedProjectRoot = useProjectStore((s) => s.selectedProjectRoot);
  const [filter, setFilter] = useState<CreatorFilter>('all');
  const [search, setSearch] = useState('');
  const [showSearch, setShowSearch] = useState(false);
  const [showNewChat, setShowNewChat] = useState(false);
  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  const [missionsOpen, setMissionsOpen] = useState(false);
  const [missions, setMissions] = useState<CronMission[]>([]);
  const [triggeringMission, setTriggeringMission] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const [newSessionIds, setNewSessionIds] = useState<Set<string>>(new Set());

  // Focus search
  useEffect(() => { if (showSearch) searchRef.current?.focus(); }, [showSearch]);

  // Fetch missions
  useEffect(() => {
    fetch('/api/missions').then(r => r.json()).then(data => {
      setMissions(Array.isArray(data) ? data : data.missions || []);
    }).catch(() => {});
  }, []);

  // Track new sessions for animation
  const prevIdsRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    const currentIds = new Set(allSessions.map((s) => s.id));
    const added = new Set<string>();
    for (const id of currentIds) {
      if (!prevIdsRef.current.has(id)) added.add(id);
    }
    prevIdsRef.current = currentIds;
    if (added.size > 0) {
      setNewSessionIds(added);
      const timer = setTimeout(() => setNewSessionIds(new Set()), 600);
      return () => clearTimeout(timer);
    }
  }, [allSessions]);

  // Filter and search
  const filtered = useMemo(() => {
    let list = allSessions;
    if (filter !== 'all') list = list.filter((s) => s.creator === filter);
    if (search.trim()) {
      const q = search.toLowerCase();
      list = list.filter((s) =>
        s.title.toLowerCase().includes(q) ||
        (s.project_name || '').toLowerCase().includes(q) ||
        (s.skill || '').toLowerCase().includes(q)
      );
    }
    return list;
  }, [allSessions, filter, search]);

  // Group by time
  const groups = useMemo(() => {
    const map = new Map<TimeGroup, SessionInfo[]>();
    for (const s of filtered) {
      const g = timeGroup(s.created_at);
      if (!map.has(g)) map.set(g, []);
      map.get(g)!.push(s);
    }
    return [...map.entries()].sort((a, b) => groupOrder[a[0]] - groupOrder[b[0]]);
  }, [filtered]);

  const handleCreateChat = useCallback((projectRoot: string) => {
    useProjectStore.getState().setSelectedProjectRoot(projectRoot);
    onCreateSession();
  }, [onCreateSession]);

  const handleTriggerMission = useCallback(async (missionId: string) => {
    setTriggeringMission(missionId);
    try {
      await fetch(`/api/missions/${missionId}/trigger`, { method: 'POST' });
      // Refresh sessions to show the new mission session
      setTimeout(() => useProjectStore.getState().fetchAllSessions(), 1000);
    } catch (e) {
      console.error('Failed to trigger mission:', e);
    } finally {
      setTriggeringMission(null);
    }
  }, []);

  const handleToggleMission = useCallback(async (missionId: string, enabled: boolean) => {
    try {
      await fetch(`/api/missions/${missionId}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ enabled }),
      });
      setMissions(prev => prev.map(m => m.id === missionId ? { ...m, enabled } : m));
    } catch (e) {
      console.error('Failed to toggle mission:', e);
    }
  }, []);

  return (
    <div className="flex flex-col h-full relative">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-slate-200 dark:border-white/5">
        <span className="text-[10px] font-bold uppercase tracking-widest text-slate-400">Sessions</span>
        <div className="flex items-center gap-1">
          <button onClick={() => setShowSearch(!showSearch)}
            className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400 hover:text-slate-600 dark:hover:text-slate-300 transition-colors"
            title="Search sessions">
            {showSearch ? <X size={13} /> : <Search size={13} />}
          </button>
          <button onClick={() => setShowNewChat(true)}
            className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400 hover:text-blue-500 transition-colors"
            title="New chat">
            <Plus size={13} />
          </button>
        </div>
      </div>

      {/* New Chat Dialog */}
      {showNewChat && (
        <NewChatDialog
          projects={projects}
          defaultProject={selectedProjectRoot}
          onClose={() => setShowNewChat(false)}
          onCreate={handleCreateChat}
        />
      )}

      {/* Search */}
      {showSearch && (
        <div className="px-3 py-1.5 border-b border-slate-200 dark:border-white/5">
          <input ref={searchRef} type="text" value={search} onChange={(e) => setSearch(e.target.value)}
            placeholder="Search sessions..."
            className="w-full text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-md px-2 py-1 outline-none focus:ring-1 focus:ring-blue-400 text-slate-700 dark:text-slate-300 placeholder-slate-400" />
        </div>
      )}

      {/* Filter tabs */}
      <div className="flex items-center gap-0.5 px-2 py-1.5 border-b border-slate-100 dark:border-white/[0.03]">
        {(['all', 'user', 'mission', 'skill'] as const).map((f) => (
          <button key={f} onClick={() => setFilter(f)}
            className={cn('px-2 py-0.5 text-[10px] font-semibold rounded-full transition-colors capitalize',
              filter === f
                ? 'bg-blue-100 dark:bg-blue-500/15 text-blue-600 dark:text-blue-400'
                : 'text-slate-400 hover:text-slate-600 dark:hover:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5')}>
            {f === 'all' ? `All (${allSessions.length})` : f}
          </button>
        ))}
      </div>

      {/* Session list (scrollable) */}
      <div className="flex-1 overflow-y-auto min-h-0">
        {groups.length === 0 && (
          <div className="flex flex-col items-center justify-center py-12 text-slate-400">
            <MessageSquare size={24} className="mb-2 opacity-30" />
            <p className="text-xs">No sessions yet</p>
            <button onClick={() => setShowNewChat(true)} className="mt-2 text-xs text-blue-500 hover:underline">
              Start a new chat
            </button>
          </div>
        )}

        {groups.map(([group, sessions]) => (
          <div key={group}>
            <div className="px-3 py-1 text-[9px] font-bold uppercase tracking-widest text-slate-400 dark:text-slate-500 bg-slate-50/80 dark:bg-white/[0.02] sticky top-0 z-10">
              {group}
            </div>
            {sessions.map((session) => {
              const isActive = session.id === activeSessionId;
              const isNew = newSessionIds.has(session.id);
              return (
                <button key={session.id} onClick={() => onSelectSession(session)}
                  className={cn(
                    'w-full flex items-start gap-2 px-3 py-2 text-left transition-all duration-150 group',
                    'hover:bg-slate-100 dark:hover:bg-white/[0.04]',
                    isActive && 'bg-blue-50 dark:bg-blue-500/[0.08] border-l-2 border-blue-500',
                    !isActive && 'border-l-2 border-transparent',
                    isNew && 'animate-slide-in',
                  )}>
                  <div className="mt-0.5">{creatorIcon(session.creator)}</div>
                  <div className="flex-1 min-w-0">
                    <span className={cn('text-xs font-medium truncate block',
                      isActive ? 'text-blue-700 dark:text-blue-300' : 'text-slate-700 dark:text-slate-300')}>
                      {session.title || session.id.slice(0, 12)}
                    </span>
                    <div className="flex items-center gap-1.5 mt-0.5">
                      {session.project_name && (
                        <span className="text-[9px] px-1 py-px rounded bg-slate-200/60 dark:bg-white/5 text-slate-500 truncate max-w-[80px]">
                          {session.project_name}
                        </span>
                      )}
                      {session.creator && session.creator !== 'user' && (
                        <span className={cn('text-[9px] px-1 py-px rounded font-medium', creatorBadge(session.creator))}>
                          {session.creator}
                        </span>
                      )}
                    </div>
                  </div>
                  <div className="flex items-center gap-1 shrink-0">
                    <span className="text-[10px] text-slate-400 tabular-nums">{relativeTime(session.created_at)}</span>
                    {onDeleteSession && (
                      <button onClick={(e) => { e.stopPropagation(); onDeleteSession(session.id); }}
                        className="p-0.5 rounded opacity-100 md:opacity-0 md:group-hover:opacity-100 hover:bg-red-100 dark:hover:bg-red-500/10 text-slate-400 hover:text-red-500 transition-all"
                        title="Delete session">
                        <Trash2 size={11} />
                      </button>
                    )}
                  </div>
                </button>
              );
            })}
          </div>
        ))}
      </div>

      {/* ─── WORKSPACE section ─── */}
      <SectionHeader title="Workspace" open={workspaceOpen} onToggle={() => setWorkspaceOpen(!workspaceOpen)} action={
        <button onClick={() => useProjectStore.getState().pickFolder()}
          className="p-0.5 rounded hover:bg-slate-200 dark:hover:bg-white/10 text-slate-400 hover:text-blue-500 transition-colors" title="Add project">
          <FolderPlus size={11} />
        </button>
      } />
      {workspaceOpen && (
        <div className="border-t border-slate-100 dark:border-white/[0.03]">
          {projects.length === 0 && (
            <div className="px-3 py-3 text-xs text-slate-400 text-center">
              No projects. <button onClick={() => useProjectStore.getState().pickFolder()} className="text-blue-500 hover:underline">Add one</button>
            </div>
          )}
          {projects.map((p) => {
            const isHome = /^\/Users\/[^/]+$/.test(p.path) || /^\/home\/[^/]+$/.test(p.path) || /^[A-Z]:\\Users\\[^\\]+$/.test(p.path) || p.path === '~';
            const name = isHome ? 'HOME' : (p.path.split('/').pop() || p.path);
            const isSelected = p.path === selectedProjectRoot;
            return (
              <div key={p.path} className={cn(
                'flex items-center gap-2 px-3 py-1.5 group',
                isSelected && 'bg-blue-50/50 dark:bg-blue-500/[0.05]'
              )}>
                <FolderOpen size={12} className={isSelected ? 'text-blue-500' : 'text-slate-400'} />
                <button onClick={() => useProjectStore.getState().setSelectedProjectRoot(p.path)}
                  className={cn('flex-1 text-left text-[11px] truncate',
                    isSelected ? 'text-blue-600 dark:text-blue-400 font-medium' : 'text-slate-600 dark:text-slate-400')}>
                  {name}
                </button>
                <button onClick={() => useProjectStore.getState().removeProject(p.path)}
                  className="p-0.5 rounded opacity-0 group-hover:opacity-100 hover:bg-red-100 dark:hover:bg-red-500/10 text-slate-400 hover:text-red-500 transition-all"
                  title="Remove project">
                  <X size={10} />
                </button>
              </div>
            );
          })}
        </div>
      )}

      {/* ─── MISSIONS section ─── */}
      <SectionHeader title="Missions" open={missionsOpen} onToggle={() => setMissionsOpen(!missionsOpen)} action={
        <div className="flex items-center gap-0.5">
          {onOpenSettings && (
            <button onClick={() => onOpenSettings('missions')}
              className="p-0.5 rounded hover:bg-slate-200 dark:hover:bg-white/10 text-slate-400 hover:text-blue-500 transition-colors" title="Mission settings">
              <Settings size={11} />
            </button>
          )}
        </div>
      } />
      {missionsOpen && (
        <div className="border-t border-slate-100 dark:border-white/[0.03]">
          {missions.length === 0 && (
            <div className="px-3 py-3 text-xs text-slate-400 text-center">
              No missions. {onOpenSettings && <button onClick={() => onOpenSettings('missions')} className="text-blue-500 hover:underline">Create one</button>}
            </div>
          )}
          {missions.map((m) => (
            <div key={m.id} className="flex items-center gap-2 px-3 py-1.5 group">
              <button onClick={() => handleToggleMission(m.id, !m.enabled)}
                className={cn('shrink-0', m.enabled ? 'text-green-500' : 'text-slate-300 dark:text-slate-600')}
                title={m.enabled ? 'Pause mission' : 'Enable mission'}>
                {m.enabled ? <Bot size={12} /> : <Pause size={12} />}
              </button>
              <div className="flex-1 min-w-0">
                <span className="text-[11px] font-medium text-slate-700 dark:text-slate-300 truncate block">{m.id}</span>
                <span className="text-[9px] text-slate-400">{m.schedule}</span>
              </div>
              <button onClick={() => handleTriggerMission(m.id)} disabled={triggeringMission === m.id}
                className="p-0.5 rounded hover:bg-green-100 dark:hover:bg-green-500/10 text-slate-400 hover:text-green-600 transition-colors disabled:opacity-40"
                title="Trigger now">
                <Play size={11} className={triggeringMission === m.id ? 'animate-spin' : ''} />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
