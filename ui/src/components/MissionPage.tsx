import React, { useState, useEffect, useCallback } from 'react';
import { ArrowLeft, Target, Plus, Trash2, Check, X, Pause, Play } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, CronMission } from '../types';
import { useSessionStore } from '../stores/sessionStore';
import { describeCron, folderLabel } from '../lib/mission-utils';
import { fetchMissions, updateMission, deleteMission } from '../lib/missions-api';
import { MissionEditor } from './mission/MissionEditor';

// Re-export for callers that import these from MissionPage.
export { MissionEditor };
export { PERMISSION_MODES } from '../lib/mission-utils';



// ---- Left Sidebar: Mission Nav ---------------------------------------------

/** Selected item in the sidebar: inline editor (existing or new), agent viewer, or nothing.
 *  Run sessions are viewed via the main-page session list, not here. */
type SidebarSelection =
  | { type: 'editor'; missionId: string | null }
  | { type: 'agent-viewer' }
  | null;

const MissionNav: React.FC<{
  missions: CronMission[];
  selection: SidebarSelection;
  onSelectMission: (m: CronMission) => void;
  onToggleEnabled: (id: string, enabled: boolean) => void;
  onDelete: (id: string) => void;
  onCreate: () => void;
}> = ({ missions, selection, onSelectMission, onToggleEnabled, onDelete, onCreate }) => {
  const enabledCount = missions.filter(m => m.enabled).length;
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  return (
    <div className="flex-1 flex flex-col min-h-0">
      {/* Header */}
      <div className="p-3 border-b border-slate-200 dark:border-white/5 flex items-center gap-2">
        <button
          onClick={onCreate}
          className="flex-1 flex items-center justify-center gap-1.5 px-3 py-1.5 text-[12px] font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 transition-colors"
        >
          <Plus size={13} /> New Mission
        </button>
        {enabledCount > 0 && (
          <span className="text-[10px] font-bold px-1.5 py-0.5 rounded-full bg-green-500/15 text-green-600 dark:text-green-400 shrink-0">
            {enabledCount} active
          </span>
        )}
      </div>

      {/* Mission list (scrollable) */}
      <div className="flex-1 overflow-y-auto p-2 space-y-1">
        {missions.length === 0 && (
          <div className="p-4 text-xs text-slate-500 italic text-center">
            No missions yet. Create one to start.
          </div>
        )}

        {missions.map(mission => {
          const projLabel = folderLabel(mission.cwd || mission.project);

          return (
            <div key={mission.id} className="rounded-lg">
              {/* Mission header. Per-mission run history was removed — run
                  sessions now live in the main page's unified session list
                  under the "Mission" tab. */}
              <div className={cn(
                'relative group rounded-lg transition-colors',
                selection?.type === 'editor' && selection.missionId === mission.id
                  ? 'bg-blue-50 dark:bg-blue-500/10'
                  : 'hover:bg-slate-50 dark:hover:bg-white/5',
              )}>
                <div className="flex items-stretch">
                  <button
                    onClick={() => onSelectMission(mission)}
                    className="flex-1 text-left px-2.5 py-2 min-w-0"
                  >
                    <div className="flex items-center gap-1.5">
                      <span className={cn(
                        'w-2 h-2 rounded-full shrink-0',
                        mission.enabled ? 'bg-green-500' : 'bg-slate-300 dark:bg-slate-600',
                      )} />
                      <span className="text-[12px] font-bold text-slate-800 dark:text-slate-200 truncate">
                        {mission.name || 'Untitled Mission'}
                      </span>
                    </div>
                    <div className="text-[11px] text-slate-400 truncate mt-0.5">
                      {describeCron(mission.schedule)}
                      {projLabel && <> &middot; {projLabel}</>}
                    </div>
                  </button>
                </div>

                {/* Action buttons on hover. Run is intentionally absent —
                    trigger missions from the main-page session list instead. */}
                <div className="absolute right-1.5 top-1.5 flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-all">
                  {confirmDeleteId === mission.id ? (
                    <>
                      <button
                        onClick={(e) => { e.stopPropagation(); onDelete(mission.id); setConfirmDeleteId(null); }}
                        className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-500/10 text-red-500"
                        title="Confirm delete"
                      >
                        <Check size={11} />
                      </button>
                      <button
                        onClick={(e) => { e.stopPropagation(); setConfirmDeleteId(null); }}
                        className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400"
                        title="Cancel"
                      >
                        <X size={11} />
                      </button>
                    </>
                  ) : (
                    <>
                      <button
                        onClick={(e) => { e.stopPropagation(); onToggleEnabled(mission.id, !mission.enabled); }}
                        className={cn(
                          'p-1 rounded transition-colors',
                          mission.enabled
                            ? 'hover:bg-amber-100 dark:hover:bg-amber-500/10 text-slate-400 hover:text-amber-500'
                            : 'hover:bg-green-100 dark:hover:bg-green-500/10 text-slate-400 hover:text-green-500',
                        )}
                        title={mission.enabled ? 'Pause — schedule kept, no runs fire' : 'Resume — re-enable the schedule'}
                        aria-label={mission.enabled ? 'Pause mission' : 'Resume mission'}
                      >
                        {mission.enabled ? <Pause size={11} /> : <Play size={11} />}
                      </button>
                      <button
                        onClick={(e) => { e.stopPropagation(); setConfirmDeleteId(mission.id); }}
                        className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-500/10 text-slate-400 hover:text-red-500"
                        title="Delete"
                      >
                        <Trash2 size={11} />
                      </button>
                    </>
                  )}
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
};


// ---- Agent Viewer (readonly) ------------------------------------------------

const AgentViewer: React.FC<{ onBack: () => void; projectRoot: string }> = ({ onBack, projectRoot }) => {
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const tryFetch = async () => {
      for (const path of ['agents/mission.md', '~/.linggen/agents/mission.md']) {
        const url = new URL('/api/agent-file', window.location.origin);
        url.searchParams.append('project_root', projectRoot);
        url.searchParams.append('path', path);
        const resp = await fetch(url.toString());
        if (resp.ok) { const data = await resp.json(); if (data.content) return data.content; }
      }
      return null;
    };
    tryFetch().then(setContent).catch(() => setContent(null)).finally(() => setLoading(false));
  }, [projectRoot]);

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="max-w-2xl mx-auto space-y-4">
        <div className="flex items-center gap-3">
          <button onClick={onBack} className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500"><ArrowLeft size={14} /></button>
          <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
            Mission Agent <span className="text-[11px] text-slate-400 font-normal ml-1">agents/mission.md</span>
          </h2>
        </div>
        {loading ? <div className="text-center py-16 text-sm text-slate-400">Loading...</div>
          : content ? <pre className="text-xs font-mono whitespace-pre-wrap bg-slate-50 dark:bg-white/[0.03] border border-slate-200 dark:border-white/10 rounded-lg p-4 overflow-x-auto text-slate-700 dark:text-slate-300">{content}</pre>
          : <div className="text-center py-16 text-sm text-slate-400">Could not load mission.md</div>
        }
      </div>
    </div>
  );
};

// ---- Right Panel: empty-state placeholder ----------------------------------
//
// The mission page no longer views run sessions inline — mission run sessions
// are in the main page's unified session list under the "Mission" tab. This
// panel just shows a placeholder when nothing is being edited.

const RightPanel: React.FC<{
  selection: SidebarSelection;
}> = ({ selection }) => {
  if (!selection) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center text-slate-400">
        <Target size={32} className="mb-3 opacity-30" />
        <p className="text-sm">Select a mission to edit, or create a new one.</p>
        <p className="text-[12px] mt-1 text-slate-400">Run sessions appear in the main page's session list under the Mission tab.</p>
      </div>
    );
  }
  return null;
};

// ---- Main Page --------------------------------------------------------------

export const MissionPage: React.FC<{
  onBack: () => void;
  projectRoot: string;
  agents: AgentInfo[];
  embedded?: boolean;
  onOpenSession?: (sessionId: string) => void;
}> = ({ onBack: rawOnBack, projectRoot, agents: _agents, embedded, onOpenSession: _onOpenSession }) => {
  // Reset mission session state when navigating away
  const onBack = useCallback(() => {
    useSessionStore.setState({ isMissionSession: false, activeMissionId: null });
    rawOnBack();
  }, [rawOnBack]);
  const [missions, setMissions] = useState<CronMission[]>([]);
  const [loading, setLoading] = useState(true);
  const [selection, setSelection] = useState<SidebarSelection>(null);
  const [rightView, setRightView] = useState<'session' | 'editor' | 'agent-viewer'>('session');
  const [editingMission, setEditingMission] = useState<CronMission | null>(null);

  // Derive unique working folders from sessions' cwd field
  const allSessions = useSessionStore((s) => s.allSessions);
  const workingFolders = React.useMemo(() => {
    const folders = new Set<string>();
    for (const s of allSessions) {
      if (s.cwd) folders.add(s.cwd);
    }
    return [...folders].sort();
  }, [allSessions]);

  const loadMissions = useCallback(async () => {
    const data = await fetchMissions();
    setMissions(data);
    setLoading(false);
  }, []);

  useEffect(() => { setLoading(true); loadMissions(); }, [loadMissions]);

  const handleToggle = async (id: string, enabled: boolean) => {
    try { await updateMission(id, { enabled }); await loadMissions(); } catch (e) { console.error('Failed to toggle mission:', e); }
  };

  const handleDelete = async (id: string) => {
    try { await deleteMission(id); await loadMissions(); if (selection && 'missionId' in selection && selection.missionId === id) setSelection(null); } catch (e) { console.error('Failed to delete mission:', e); }
  };

  const handleSelectMission = (m: CronMission) => {
    setEditingMission(m);
    setSelection({ type: 'editor', missionId: m.id });
    setRightView('editor');
  };

  const handleCreate = () => {
    setEditingMission(null);
    setSelection({ type: 'editor', missionId: null });
    setRightView('editor');
  };

  const handleSave = async (mission: CronMission) => {
    await loadMissions();
    setEditingMission(mission);
    setSelection({ type: 'editor', missionId: mission.id });
    setRightView('editor');
  };

  const handleCancel = () => {
    setEditingMission(null);
    setSelection(null);
    setRightView('session');
  };

  const handleViewAgent = () => {
    setSelection({ type: 'agent-viewer' });
    setRightView('agent-viewer');
  };

  const enabledCount = missions.filter(m => m.enabled).length;

  const mainContent = (
    <div className="flex-1 flex overflow-hidden">
      {/* Left sidebar — mission nav */}
      <div className="w-72 border-r border-slate-200 dark:border-white/5 flex flex-col bg-white dark:bg-[#0f0f0f] h-full">
        <MissionNav
          missions={missions}
          selection={selection}
          onSelectMission={handleSelectMission}
          onToggleEnabled={handleToggle}
          onDelete={handleDelete}
          onCreate={handleCreate}
        />
      </div>

      {/* Right panel — session content or full-page editor */}
      <main className="flex-1 flex flex-col overflow-hidden bg-slate-100/40 dark:bg-[#0a0a0a] min-h-0">
        {loading ? (
          <div className="flex-1 flex items-center justify-center text-sm text-slate-400">Loading...</div>
        ) : rightView === 'editor' ? (
          <MissionEditor
            editing={editingMission}
            workingFolders={workingFolders}
            onSave={handleSave}
            onCancel={handleCancel}
            onViewAgent={handleViewAgent}
          />
        ) : rightView === 'agent-viewer' ? (
          <AgentViewer
            onBack={() => { setRightView(editingMission ? 'editor' : 'session'); }}
            projectRoot={projectRoot}
          />
        ) : (
          <RightPanel selection={selection} />
        )}
      </main>
    </div>
  );

  if (embedded) {
    return <div className="flex flex-col h-full">{mainContent}</div>;
  }

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200">
      <header className="flex items-center gap-4 px-6 py-3 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md shrink-0">
        <button onClick={onBack} className="p-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500 transition-colors">
          <ArrowLeft size={16} />
        </button>
        <div className="flex items-center gap-2">
          <Target size={18} className={enabledCount > 0 ? 'text-green-500' : 'text-slate-400'} />
          <h1 className="text-lg font-bold tracking-tight">Missions</h1>
        </div>
        {enabledCount > 0 && (
          <span className="text-[11px] font-bold uppercase tracking-wide px-2 py-0.5 rounded-full bg-green-500/15 text-green-600 dark:text-green-400">
            {enabledCount} active
          </span>
        )}
      </header>
      {mainContent}
    </div>
  );
};
