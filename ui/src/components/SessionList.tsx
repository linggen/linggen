/**
 * Unified left panel — sessions and missions.
 *
 * Two sections:
 * 1. SESSIONS — chronological list with filters and time grouping
 * 2. MISSIONS — mission list with inline trigger
 */
import React, { useState, useMemo, useRef, useEffect, useCallback } from 'react';
import {
  MessageSquare, Bot, Sparkles, Plus, Search, X, Trash2,
  Play, Pause, ChevronDown, ChevronRight, Settings, RefreshCw,
  CheckSquare, Loader2,
} from 'lucide-react';
import { cn } from '../lib/cn';
import type { SessionInfo, CronMission } from '../types';
import { useSessionStore } from '../stores/sessionStore';
import { useServerStore } from '../stores/serverStore';
import { useOpenMissionEditor } from '../hooks/useOpenMissionEditor';

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
      <span className="text-[10px] font-bold uppercase tracking-widest text-slate-400">{title}</span>
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
  /** When provided, overrides the sessions from the store (used by consumer mode to filter). */
  filterSessions?: SessionInfo[];
  /** Hide the Missions section at the bottom (skill iframes don't need it). */
  hideMissions?: boolean;
  /** Hide the user/mission/skill/all filter tab row (skill iframes pin one creator). */
  hideFilters?: boolean;
}> = ({ activeSessionId, onSelectSession, onCreateSession, onDeleteSession, onOpenSettings, filterSessions, hideMissions, hideFilters }) => {
  const storeSessions = useSessionStore((s) => s.allSessions);
  const allSessions = filterSessions ?? storeSessions;
  const agentStatus = useServerStore((s) => s.agentStatus);
  // Server-authoritative "is a run in flight" per session. Indexed
  // once per render: O(runs) instead of a scan per row. Subagent runs
  // (parent_run_id set) intentionally don't light up the row spinner
  // — that's the SubagentPane's signal, not the session list's.
  const agentRunsRaw = useServerStore((s) => s.agentRuns);
  const runningSessionIds = useMemo(() => {
    const out = new Set<string>();
    for (const r of agentRunsRaw) {
      if (r.status === 'running' && !r.parent_run_id) out.add(r.session_id);
    }
    return out;
  }, [agentRunsRaw]);
  const openMissionEditor = useOpenMissionEditor();
  // When filters are hidden the consumer is pre-filtering via filterSessions,
  // so default to 'all' to avoid the 'user'-only default clipping the list.
  const [filter, setFilter] = useState<CreatorFilter>(hideFilters ? 'all' : 'user');
  const [search, setSearch] = useState('');
  const [showSearch, setShowSearch] = useState(false);
  const [missionsOpen, setMissionsOpen] = useState(false);
  const [missions, setMissions] = useState<CronMission[]>([]);
  const [triggeringMission, setTriggeringMission] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const [newSessionIds, setNewSessionIds] = useState<Set<string>>(new Set());
  const [selectMode, setSelectMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

  // Focus search
  useEffect(() => { if (showSearch) searchRef.current?.focus(); }, [showSearch]);

  // Fetch missions
  useEffect(() => {
    fetch('/api/missions').then(r => r.json()).then(data => {
      setMissions(Array.isArray(data) ? data : data.missions || []);
    }).catch(() => {});
  }, []);

  // Track new sessions for animation. Re-baseline silently on the first
  // populated load (page_state event after mount) so every row doesn't
  // slide in on refresh.
  const prevIdsRef = useRef<Set<string>>(new Set());
  const didInitIdsRef = useRef(false);
  useEffect(() => {
    const currentIds = new Set(allSessions.map((s) => s.id));
    if (!didInitIdsRef.current) {
      if (currentIds.size > 0) didInitIdsRef.current = true;
      prevIdsRef.current = currentIds;
      return;
    }
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

  // Per-creator counts (computed before search/filter so tab counts reflect the full set)
  const counts = useMemo(() => {
    let user = 0, mission = 0, skill = 0;
    for (const s of allSessions) {
      if (s.creator === 'mission') mission++;
      else if (s.creator === 'skill') skill++;
      else user++;
    }
    return { user, mission, skill, all: allSessions.length };
  }, [allSessions]);

  // Animate the tab count (e.g. "Mission (3)") with a +1 badge when it grows.
  // bumpKey is a counter that re-mounts the animation node so consecutive
  // increments still trigger fresh keyframes.
  const [bumpKey, setBumpKey] = useState<Record<CreatorFilter, number>>({ all: 0, user: 0, mission: 0, skill: 0 });
  const prevCountsRef = useRef(counts);
  // Sessions load asynchronously after mount via the page_state event. The first
  // populated update would otherwise look like growth on every tab. Re-baseline
  // silently until we've observed real data at least once.
  const didInitCountsRef = useRef(false);
  useEffect(() => {
    if (!didInitCountsRef.current) {
      if (counts.all > 0) didInitCountsRef.current = true;
      prevCountsRef.current = counts;
      return;
    }
    const prev = prevCountsRef.current;
    const grew: CreatorFilter[] = [];
    (['user', 'mission', 'skill', 'all'] as CreatorFilter[]).forEach((k) => {
      if (counts[k] > prev[k]) grew.push(k);
    });
    if (grew.length > 0) {
      setBumpKey((b) => {
        const next = { ...b };
        for (const k of grew) next[k] = b[k] + 1;
        return next;
      });
    }
    prevCountsRef.current = counts;
  }, [counts]);

  // Filter and search
  const filtered = useMemo(() => {
    let list = allSessions;
    if (filter === 'user') list = list.filter((s) => !s.creator || s.creator === 'user');
    else if (filter !== 'all') list = list.filter((s) => s.creator === filter);
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



  const handleTriggerMission = useCallback(async (missionId: string) => {
    setTriggeringMission(missionId);
    // Spin for at least 1s so the click is visibly acknowledged even if the
    // API returns instantly. Matches the mission-settings page pattern.
    const minSpin = new Promise(res => setTimeout(res, 1000));
    try {
      // Axum's Json<TriggerMissionRequest> extractor needs a real JSON body
      // and Content-Type — a bodyless POST 415s silently.
      const resp = await fetch(`/api/missions/${missionId}/trigger`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      });
      if (!resp.ok) {
        console.error('Failed to trigger mission:', resp.status, await resp.text());
      }
      // New session appears via SessionCreated event; refresh just in case.
      setTimeout(() => useSessionStore.getState().fetchAllSessions(), 1000);
    } catch (e) {
      console.error('Failed to trigger mission:', e);
    } finally {
      await minSpin;
      setTriggeringMission(null);
    }
  }, []);

  const exitSelectMode = useCallback(() => {
    setSelectMode(false);
    setSelectedIds(new Set());
  }, []);

  const toggleSelectMode = useCallback(() => {
    if (selectMode) exitSelectMode();
    else setSelectMode(true);
  }, [selectMode, exitSelectMode]);

  const toggleSessionSelected = useCallback((id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);

  const toggleGroupSelected = useCallback((groupIds: string[]) => {
    setSelectedIds((prev) => {
      const allSelected = groupIds.every((id) => prev.has(id));
      const next = new Set(prev);
      if (allSelected) groupIds.forEach((id) => next.delete(id));
      else groupIds.forEach((id) => next.add(id));
      return next;
    });
  }, []);

  const handleBulkDelete = useCallback(async () => {
    const ids = [...selectedIds];
    if (ids.length === 0) return;
    await useSessionStore.getState().removeSessions(ids);
    exitSelectMode();
  }, [selectedIds, exitSelectMode]);

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
    <div className="flex flex-col flex-1 min-h-0 relative">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-slate-200 dark:border-white/5">
        <span className="text-[11px] font-bold uppercase tracking-widest text-slate-400">Sessions</span>
        <div className="flex items-center gap-1">
          <button onClick={() => { useSessionStore.getState().fetchAllSessions(); }}
            className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400 hover:text-slate-600 dark:hover:text-slate-300 transition-colors"
            title="Refresh sessions">
            <RefreshCw size={13} />
          </button>
          <button onClick={() => setShowSearch(!showSearch)}
            className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400 hover:text-slate-600 dark:hover:text-slate-300 transition-colors"
            title="Search sessions">
            {showSearch ? <X size={13} /> : <Search size={13} />}
          </button>
          <button onClick={toggleSelectMode}
            className={cn(
              'p-1 rounded transition-colors',
              selectMode
                ? 'bg-blue-100 dark:bg-blue-500/15 text-blue-600 dark:text-blue-400'
                : 'hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400 hover:text-slate-600 dark:hover:text-slate-300',
            )}
            title={selectMode ? 'Exit select mode' : 'Select sessions'}>
            <CheckSquare size={13} />
          </button>
          <button onClick={onCreateSession}
            className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400 hover:text-blue-500 transition-colors"
            title="New chat">
            <Plus size={13} />
          </button>
        </div>
      </div>

      {/* Search */}
      {showSearch && (
        <div className="px-3 py-1.5 border-b border-slate-200 dark:border-white/5">
          <input ref={searchRef} type="text" value={search} onChange={(e) => setSearch(e.target.value)}
            placeholder="Search sessions..."
            className="w-full text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-md px-2 py-1 outline-none focus:ring-1 focus:ring-blue-400 text-slate-700 dark:text-slate-300 placeholder-slate-400" />
        </div>
      )}

      {/* Filter tabs */}
      {!hideFilters && (
      <div className="flex items-center gap-0.5 px-2 py-1.5 border-b border-slate-100 dark:border-white/[0.03]">
        {(['user', 'mission', 'skill', 'all'] as const).map((f) => {
          const k = bumpKey[f];
          return (
            <button key={f} onClick={() => setFilter(f)}
              className={cn('relative px-2 py-0.5 text-[11px] font-semibold rounded-full transition-colors capitalize whitespace-nowrap',
                filter === f
                  ? 'bg-blue-100 dark:bg-blue-500/15 text-blue-600 dark:text-blue-400'
                  : 'text-slate-400 hover:text-slate-600 dark:hover:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5')}>
              {f === 'all' ? 'All' : f}
              {' ('}
              <span key={k} className={cn('inline-block tabular-nums', k > 0 && 'animate-count-bump')}>
                {counts[f]}
              </span>
              {')'}
              {k > 0 && (
                <span key={`plus-${k}`}
                  className="absolute left-1/2 -top-2 text-[16px] font-extrabold text-emerald-500 drop-shadow-[0_1px_2px_rgba(16,185,129,0.45)] pointer-events-none animate-plus-one-fly">
                  +1
                </span>
              )}
            </button>
          );
        })}
      </div>
      )}

      {/* Select-mode action bar */}
      {selectMode && (
        <div className="flex items-center justify-between gap-2 px-3 py-1.5 border-b border-slate-200 dark:border-white/5 bg-blue-50/60 dark:bg-blue-500/[0.08]">
          <span className="text-[11px] font-semibold text-blue-700 dark:text-blue-300 tabular-nums">
            {selectedIds.size} selected
          </span>
          <div className="flex items-center gap-1">
            <button
              onClick={handleBulkDelete}
              disabled={selectedIds.size === 0}
              className={cn(
                'flex items-center gap-1 px-2 py-0.5 text-[11px] font-semibold rounded transition-colors',
                selectedIds.size === 0
                  ? 'text-slate-400 cursor-not-allowed'
                  : 'text-red-600 hover:bg-red-100 dark:text-red-400 dark:hover:bg-red-500/10',
              )}
              title="Delete selected sessions">
              <Trash2 size={11} />
              Delete
            </button>
            <button
              onClick={exitSelectMode}
              className="px-2 py-0.5 text-[11px] font-semibold rounded text-slate-500 hover:bg-slate-200 dark:text-slate-400 dark:hover:bg-white/10 transition-colors"
              title="Cancel selection">
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Session list (scrollable) */}
      <div className="flex-1 overflow-y-auto min-h-0">
        {groups.length === 0 && (
          <div className="flex flex-col items-center justify-center py-12 text-slate-400">
            <MessageSquare size={24} className="mb-2 opacity-30" />
            <p className="text-xs">No sessions yet</p>
            <button onClick={onCreateSession} className="mt-2 text-xs text-blue-500 hover:underline">
              Start a new chat
            </button>
          </div>
        )}

        {groups.map(([group, sessions]) => {
          const groupIds = sessions.map((s) => s.id);
          const groupSelectedCount = groupIds.filter((id) => selectedIds.has(id)).length;
          const groupAllSelected = groupSelectedCount === groupIds.length && groupIds.length > 0;
          const groupSomeSelected = groupSelectedCount > 0 && !groupAllSelected;
          return (
          <div key={group}>
            <div className="flex items-center gap-2 px-3 py-1 bg-slate-50/80 dark:bg-white/[0.02] sticky top-0 z-10">
              {selectMode && (
                <input
                  type="checkbox"
                  className="w-3 h-3 accent-blue-500 cursor-pointer"
                  checked={groupAllSelected}
                  ref={(el) => { if (el) el.indeterminate = groupSomeSelected; }}
                  onChange={() => toggleGroupSelected(groupIds)}
                  onClick={(e) => e.stopPropagation()}
                  title={groupAllSelected ? `Deselect ${group}` : `Select all in ${group}`}
                />
              )}
              <span className="text-[10px] font-bold uppercase tracking-widest text-slate-400 dark:text-slate-500">
                {group}
              </span>
              {selectMode && groupSelectedCount > 0 && (
                <span className="text-[10px] text-blue-500 tabular-nums">{groupSelectedCount}/{groupIds.length}</span>
              )}
            </div>
            {sessions.map((session) => {
              const isActive = session.id === activeSessionId;
              const isNew = newSessionIds.has(session.id);
              const isChecked = selectedIds.has(session.id);
              const handleRowClick = () => {
                if (selectMode) toggleSessionSelected(session.id);
                else onSelectSession(session);
              };
              return (
                <div key={session.id} onClick={handleRowClick} role="button" tabIndex={0}
                  onKeyDown={(e) => { if (e.key === 'Enter') handleRowClick(); }}
                  className={cn(
                    'w-full flex items-start gap-2 px-3 py-2 text-left transition-all duration-150 group cursor-pointer',
                    'hover:bg-slate-100 dark:hover:bg-white/[0.04]',
                    isActive && !selectMode && 'bg-blue-50 dark:bg-blue-500/[0.08] border-l-2 border-blue-500',
                    (!isActive || selectMode) && 'border-l-2 border-transparent',
                    selectMode && isChecked && 'bg-blue-50 dark:bg-blue-500/[0.08]',
                    isNew && 'animate-slide-in',
                  )}>
                  {selectMode ? (
                    <input
                      type="checkbox"
                      className="mt-1 w-3 h-3 accent-blue-500 cursor-pointer shrink-0"
                      checked={isChecked}
                      onChange={() => toggleSessionSelected(session.id)}
                      onClick={(e) => e.stopPropagation()}
                    />
                  ) : (
                    <div className="mt-0.5">
                      {runningSessionIds.has(session.id)
                        ? <div className="w-[13px] h-[13px] rounded-full border-2 border-blue-500 border-t-transparent animate-spin shrink-0" />
                        : creatorIcon(session.creator)}
                    </div>
                  )}
                  <div className="flex-1 min-w-0">
                    <span className={cn('text-xs font-medium truncate block',
                      isActive && !selectMode ? 'text-blue-700 dark:text-blue-300' : 'text-slate-700 dark:text-slate-300')}>
                      {session.title || session.id.slice(0, 12)}
                    </span>
                    <div className="flex items-center gap-1.5 mt-0.5">
                      {session.project_name && (
                        <span className="text-[10px] px-1 py-px rounded bg-slate-200/60 dark:bg-white/5 text-slate-500 truncate max-w-[80px]">
                          {session.project_name}
                        </span>
                      )}
                      {session.creator && session.creator !== 'user' && (
                        <span className={cn('text-[10px] px-1 py-px rounded font-medium', creatorBadge(session.creator))}>
                          {session.creator}
                        </span>
                      )}
                    </div>
                  </div>
                  <div className="flex items-center gap-1 shrink-0">
                    <span className="text-[11px] text-slate-400 tabular-nums">{relativeTime(session.created_at)}</span>
                    {!selectMode && onDeleteSession && (
                      <button onClick={(e) => { e.stopPropagation(); onDeleteSession(session.id); }}
                        className="p-0.5 rounded opacity-100 md:opacity-0 md:group-hover:opacity-100 hover:bg-red-100 dark:hover:bg-red-500/10 text-slate-400 hover:text-red-500 transition-all"
                        title="Delete session">
                        <Trash2 size={11} />
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
          );
        })}
      </div>

      {/* ─── MISSIONS section ─── */}
      {!hideMissions && (<>
      <SectionHeader title="Missions" open={missionsOpen} onToggle={() => setMissionsOpen(!missionsOpen)} action={
        <div className="flex items-center gap-0.5">
          {onOpenSettings && (
            <button onClick={() => onOpenSettings('mission')}
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
              No missions. <button onClick={() => openMissionEditor(null)} className="text-blue-500 hover:underline">Create one</button>
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
                <span className="text-[12px] font-medium text-slate-700 dark:text-slate-300 truncate block">{m.id}</span>
                <span className="text-[10px] text-slate-400">{m.schedule}</span>
              </div>
              <button onClick={() => handleTriggerMission(m.id)} disabled={triggeringMission === m.id}
                className={cn(
                  'p-0.5 rounded transition-colors',
                  triggeringMission === m.id
                    ? 'text-green-600 bg-green-100 dark:bg-green-500/15'
                    : 'text-slate-400 hover:text-green-600 hover:bg-green-100 dark:hover:bg-green-500/10',
                )}
                title="Trigger now">
                {triggeringMission === m.id
                  ? <Loader2 size={11} className="animate-spin" />
                  : <Play size={11} />}
              </button>
            </div>
          ))}
        </div>
      )}
      </>)}
    </div>
  );
};
