import React, { useState, useEffect, useCallback } from 'react';
import { ArrowLeft, Target, Plus, Play, Pause, Trash2, Clock, History, Edit3, Check, X } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, CronMission, MissionRunEntry, MissionTab } from '../types';

const formatTimestamp = (ts: number) => {
  if (!ts || ts <= 0) return '-';
  const d = new Date(ts * 1000);
  return d.toLocaleString();
};

const timeSince = (ts: number) => {
  if (!ts || ts <= 0) return '';
  const now = Date.now();
  const diffMs = now - ts * 1000;
  if (diffMs < 0) return '';
  const mins = Math.floor(diffMs / 60_000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
};

/** Human-readable description of a cron schedule. */
const describeCron = (schedule: string): string => {
  const parts = schedule.split(/\s+/);
  if (parts.length !== 5) return schedule;
  const [min, hour, dom, mon, dow] = parts;

  if (min === '*' && hour === '*' && dom === '*' && mon === '*' && dow === '*') return 'Every minute';
  if (min.startsWith('*/') && hour === '*' && dom === '*' && mon === '*' && dow === '*') {
    return `Every ${min.slice(2)} minutes`;
  }
  if (hour.startsWith('*/') && dom === '*' && mon === '*' && dow === '*') {
    return `Every ${hour.slice(2)} hours at minute ${min}`;
  }
  if (dom === '*' && mon === '*' && dow === '*') {
    return `Daily at ${hour}:${min.padStart(2, '0')}`;
  }
  if (dom === '*' && mon === '*' && dow !== '*') {
    const dayNames: Record<string, string> = { '0': 'Sun', '1': 'Mon', '2': 'Tue', '3': 'Wed', '4': 'Thu', '5': 'Fri', '6': 'Sat', '7': 'Sun' };
    const days = dow.split(',').map(d => dayNames[d] || d).join(', ');
    if (dow.includes('-')) {
      const [start, end] = dow.split('-');
      return `${dayNames[start] || start}-${dayNames[end] || end} at ${hour}:${min.padStart(2, '0')}`;
    }
    return `${days} at ${hour}:${min.padStart(2, '0')}`;
  }
  return schedule;
};

// --- API helpers ---

async function fetchMissions(projectRoot: string): Promise<CronMission[]> {
  const url = new URL('/api/missions', window.location.origin);
  url.searchParams.append('project_root', projectRoot);
  const resp = await fetch(url.toString());
  if (!resp.ok) return [];
  const data = await resp.json();
  return Array.isArray(data.missions) ? data.missions : [];
}

async function createMission(projectRoot: string, schedule: string, agentId: string, prompt: string, model?: string): Promise<CronMission | null> {
  const resp = await fetch('/api/missions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ project_root: projectRoot, schedule, agent_id: agentId, prompt, model: model || null }),
  });
  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(text);
  }
  return resp.json();
}

async function updateMission(id: string, projectRoot: string, updates: Record<string, any>): Promise<CronMission | null> {
  const resp = await fetch(`/api/missions/${encodeURIComponent(id)}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ project_root: projectRoot, ...updates }),
  });
  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(text);
  }
  return resp.json();
}

async function deleteMission(id: string, projectRoot: string): Promise<void> {
  const url = new URL(`/api/missions/${encodeURIComponent(id)}`, window.location.origin);
  url.searchParams.append('project_root', projectRoot);
  await fetch(url.toString(), { method: 'DELETE' });
}

async function fetchMissionRuns(id: string, projectRoot: string): Promise<MissionRunEntry[]> {
  const url = new URL(`/api/missions/${encodeURIComponent(id)}/runs`, window.location.origin);
  url.searchParams.append('project_root', projectRoot);
  const resp = await fetch(url.toString());
  if (!resp.ok) return [];
  const data = await resp.json();
  return Array.isArray(data.runs) ? data.runs : [];
}

// --- Mission List ---

const MissionCard: React.FC<{
  mission: CronMission;
  projectRoot: string;
  onToggle: (id: string, enabled: boolean) => void;
  onEdit: (m: CronMission) => void;
  onDelete: (id: string) => void;
  onViewRuns: (m: CronMission) => void;
}> = ({ mission, projectRoot: _projectRoot, onToggle, onEdit, onDelete, onViewRuns }) => {
  const [confirmDelete, setConfirmDelete] = useState(false);

  return (
    <div className={cn(
      'border rounded-lg p-4 bg-white dark:bg-white/[0.02] transition-colors',
      mission.enabled
        ? 'border-green-500/20'
        : 'border-slate-200 dark:border-white/10 opacity-60',
    )}>
      <div className="flex items-start justify-between gap-3 mb-2">
        <div className="flex items-center gap-2 min-w-0">
          <button
            onClick={() => onToggle(mission.id, !mission.enabled)}
            className={cn(
              'w-8 h-5 rounded-full relative transition-colors shrink-0',
              mission.enabled ? 'bg-green-500' : 'bg-slate-300 dark:bg-slate-600',
            )}
          >
            <span className={cn(
              'absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform',
              mission.enabled ? 'left-3.5' : 'left-0.5',
            )} />
          </button>
          <span className="text-xs font-mono text-blue-600 dark:text-blue-400 bg-blue-500/10 px-2 py-0.5 rounded">
            {mission.schedule}
          </span>
          <span className="text-[10px] text-slate-500 truncate">{describeCron(mission.schedule)}</span>
        </div>
        <div className="flex items-center gap-1 shrink-0">
          <button
            onClick={() => onViewRuns(mission)}
            className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400 hover:text-slate-600"
            title="View runs"
          >
            <History size={14} />
          </button>
          <button
            onClick={() => onEdit(mission)}
            className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400 hover:text-slate-600"
            title="Edit"
          >
            <Edit3 size={14} />
          </button>
          {confirmDelete ? (
            <div className="flex items-center gap-0.5">
              <button
                onClick={() => { onDelete(mission.id); setConfirmDelete(false); }}
                className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-500/10 text-red-500"
                title="Confirm delete"
              >
                <Check size={14} />
              </button>
              <button
                onClick={() => setConfirmDelete(false)}
                className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400"
                title="Cancel"
              >
                <X size={14} />
              </button>
            </div>
          ) : (
            <button
              onClick={() => setConfirmDelete(true)}
              className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-500/10 text-slate-400 hover:text-red-500"
              title="Delete"
            >
              <Trash2 size={14} />
            </button>
          )}
        </div>
      </div>

      <div className="flex items-center gap-2 mb-2">
        <span className="text-[10px] font-bold uppercase tracking-wide px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-600 dark:text-purple-400">
          {mission.agent_id}
        </span>
        {mission.model && (
          <span className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-slate-100 dark:bg-white/5 text-slate-500">
            {mission.model}
          </span>
        )}
        <span className="text-[10px] text-slate-400 ml-auto">
          Created {timeSince(mission.created_at)}
        </span>
      </div>

      <div className="text-xs text-slate-700 dark:text-slate-300 line-clamp-2 whitespace-pre-wrap">
        {mission.prompt}
      </div>
    </div>
  );
};

const MissionList: React.FC<{
  missions: CronMission[];
  projectRoot: string;
  onToggle: (id: string, enabled: boolean) => void;
  onEdit: (m: CronMission) => void;
  onDelete: (id: string) => void;
  onViewRuns: (m: CronMission) => void;
  onCreate: () => void;
}> = ({ missions, projectRoot, onToggle, onEdit, onDelete, onViewRuns, onCreate }) => {
  const enabledCount = missions.filter(m => m.enabled).length;

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
            Missions ({missions.length})
          </h2>
          {enabledCount > 0 && (
            <span className="text-[10px] font-bold px-2 py-0.5 rounded-full bg-green-500/15 text-green-600 dark:text-green-400">
              {enabledCount} active
            </span>
          )}
        </div>
        <button
          onClick={onCreate}
          className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 transition-colors"
        >
          <Plus size={14} /> New Mission
        </button>
      </div>

      {missions.length === 0 ? (
        <div className="text-center py-16">
          <Target size={32} className="mx-auto text-slate-300 dark:text-slate-600 mb-3" />
          <p className="text-sm text-slate-500">No missions yet</p>
          <p className="text-[11px] text-slate-400 mt-1">Create a mission to schedule agent tasks</p>
        </div>
      ) : (
        <div className="space-y-3">
          {missions.map(m => (
            <MissionCard
              key={m.id}
              mission={m}
              projectRoot={projectRoot}
              onToggle={onToggle}
              onEdit={onEdit}
              onDelete={onDelete}
              onViewRuns={onViewRuns}
            />
          ))}
        </div>
      )}
    </div>
  );
};

// --- Mission Editor (Create/Edit) ---

const CRON_PRESETS = [
  { label: 'Every 30 min', value: '*/30 * * * *' },
  { label: 'Every hour', value: '0 * * * *' },
  { label: 'Every 2 hours', value: '0 */2 * * *' },
  { label: 'Daily at 9am', value: '0 9 * * *' },
  { label: 'Weekdays 9am', value: '0 9 * * 1-5' },
  { label: 'Weekly Sunday', value: '0 0 * * 0' },
];

const MissionEditor: React.FC<{
  editing: CronMission | null;
  agents: AgentInfo[];
  projectRoot: string;
  onSave: (mission: CronMission) => void;
  onCancel: () => void;
}> = ({ editing, agents, projectRoot, onSave, onCancel }) => {
  const [schedule, setSchedule] = useState(editing?.schedule || '*/30 * * * *');
  const [agentId, setAgentId] = useState(editing?.agent_id || (agents[0]?.name || 'ling'));
  const [prompt, setPrompt] = useState(editing?.prompt || '');
  const [model, setModel] = useState(editing?.model || '');
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const handleSave = async () => {
    if (!schedule.trim() || !prompt.trim()) {
      setError('Schedule and prompt are required');
      return;
    }
    setSaving(true);
    setError(null);
    try {
      let result: CronMission | null;
      if (editing) {
        result = await updateMission(editing.id, projectRoot, {
          schedule,
          agent_id: agentId,
          prompt,
          model: model || null,
        });
      } else {
        result = await createMission(projectRoot, schedule, agentId, prompt, model || undefined);
      }
      if (result) onSave(result);
    } catch (e: any) {
      setError(e.message || 'Failed to save mission');
    }
    setSaving(false);
  };

  return (
    <div className="space-y-4">
      <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
        {editing ? 'Edit Mission' : 'New Mission'}
      </h2>

      {error && (
        <div className="bg-red-500/10 border border-red-500/20 rounded-lg p-3 text-xs text-red-600 dark:text-red-400">
          {error}
        </div>
      )}

      {/* Schedule */}
      <div>
        <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
          Cron Schedule
        </label>
        <input
          type="text"
          value={schedule}
          onChange={(e) => setSchedule(e.target.value)}
          placeholder="*/30 * * * *"
          className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30"
        />
        <div className="flex flex-wrap gap-1.5 mt-2">
          {CRON_PRESETS.map(p => (
            <button
              key={p.value}
              onClick={() => setSchedule(p.value)}
              className={cn(
                'text-[10px] px-2 py-0.5 rounded-full border transition-colors',
                schedule === p.value
                  ? 'border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-400'
                  : 'border-slate-200 dark:border-white/10 text-slate-500 hover:bg-slate-50 dark:hover:bg-white/5',
              )}
            >
              {p.label}
            </button>
          ))}
        </div>
        <div className="text-[10px] text-slate-400 mt-1.5">
          {describeCron(schedule)}
        </div>
      </div>

      {/* Agent */}
      <div>
        <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
          Agent
        </label>
        <select
          value={agentId}
          onChange={(e) => setAgentId(e.target.value)}
          className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30"
        >
          {agents.map(a => (
            <option key={a.name} value={a.name}>{a.name} — {a.description}</option>
          ))}
        </select>
      </div>

      {/* Prompt */}
      <div>
        <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
          Prompt
        </label>
        <textarea
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder="The instruction to send to the agent on each trigger..."
          rows={6}
          className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 resize-y focus:outline-none focus:ring-2 focus:ring-blue-500/30"
        />
      </div>

      {/* Model override (optional) */}
      <div>
        <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
          Model Override <span className="text-slate-400">(optional)</span>
        </label>
        <input
          type="text"
          value={model}
          onChange={(e) => setModel(e.target.value)}
          placeholder="Leave empty to use agent default"
          className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30"
        />
      </div>

      {/* Actions */}
      <div className="flex items-center gap-3 pt-2">
        <button
          onClick={handleSave}
          disabled={saving || !prompt.trim()}
          className="px-4 py-2 text-sm font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
        >
          {saving ? 'Saving...' : editing ? 'Update Mission' : 'Create Mission'}
        </button>
        <button
          onClick={onCancel}
          className="px-4 py-2 text-sm font-semibold rounded-lg border border-slate-200 dark:border-white/10 text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5 transition-colors"
        >
          Cancel
        </button>
      </div>
    </div>
  );
};

// --- Mission Run History ---

const RunsView: React.FC<{
  mission: CronMission;
  projectRoot: string;
  onBack: () => void;
}> = ({ mission, projectRoot, onBack }) => {
  const [runs, setRuns] = useState<MissionRunEntry[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetchMissionRuns(mission.id, projectRoot)
      .then(setRuns)
      .finally(() => setLoading(false));
  }, [mission.id, projectRoot]);

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <button
          onClick={onBack}
          className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500"
        >
          <ArrowLeft size={14} />
        </button>
        <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
          Run History: <span className="font-mono text-blue-600 dark:text-blue-400">{mission.schedule}</span>
        </h2>
      </div>

      <div className="text-xs text-slate-500 mb-2">
        Agent: <span className="font-semibold">{mission.agent_id}</span> — {mission.prompt.slice(0, 100)}{mission.prompt.length > 100 ? '...' : ''}
      </div>

      {loading ? (
        <div className="text-center py-16 text-sm text-slate-400">Loading...</div>
      ) : runs.length === 0 ? (
        <div className="text-center py-16">
          <Clock size={32} className="mx-auto text-slate-300 dark:text-slate-600 mb-3" />
          <p className="text-sm text-slate-500">No runs yet</p>
        </div>
      ) : (
        <div className="space-y-1.5">
          {[...runs].reverse().map((run, i) => (
            <div
              key={`${run.run_id}-${i}`}
              className={cn(
                'flex items-center gap-3 px-3 py-2 rounded-lg border',
                run.skipped
                  ? 'bg-amber-50/50 dark:bg-amber-500/5 border-amber-200 dark:border-amber-500/10'
                  : run.status === 'completed'
                    ? 'bg-white dark:bg-white/[0.02] border-green-200 dark:border-green-500/10'
                    : 'bg-white dark:bg-white/[0.02] border-slate-200 dark:border-white/10',
              )}
            >
              <span className="text-[10px] text-slate-400 font-mono shrink-0 w-36">
                {formatTimestamp(run.triggered_at)}
              </span>
              <span className={cn(
                'text-[10px] font-bold px-1.5 py-0.5 rounded uppercase tracking-wide',
                run.skipped
                  ? 'bg-amber-500/15 text-amber-600'
                  : run.status === 'completed'
                    ? 'bg-green-500/15 text-green-600'
                    : run.status === 'failed'
                      ? 'bg-red-500/15 text-red-600'
                      : 'bg-slate-500/15 text-slate-500',
              )}>
                {run.skipped ? 'skipped' : run.status}
              </span>
              {run.run_id && !run.skipped && (
                <span className="text-[10px] text-slate-400 font-mono truncate">
                  {run.run_id}
                </span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

// --- Main Page ---

export const MissionPage: React.FC<{
  onBack: () => void;
  projectRoot: string;
  agents: AgentInfo[];
  embedded?: boolean;
}> = ({ onBack, projectRoot, agents, embedded }) => {
  const [tab, setTab] = useState<MissionTab>('list');
  const [missions, setMissions] = useState<CronMission[]>([]);
  const [loading, setLoading] = useState(true);
  const [editingMission, setEditingMission] = useState<CronMission | null>(null);
  const [viewingRunsMission, setViewingRunsMission] = useState<CronMission | null>(null);

  const loadMissions = useCallback(async () => {
    const data = await fetchMissions(projectRoot);
    setMissions(data);
    setLoading(false);
  }, [projectRoot]);

  useEffect(() => {
    loadMissions();
  }, [loadMissions]);

  const handleToggle = async (id: string, enabled: boolean) => {
    try {
      await updateMission(id, projectRoot, { enabled });
      await loadMissions();
    } catch (e) {
      console.error('Failed to toggle mission:', e);
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteMission(id, projectRoot);
      await loadMissions();
    } catch (e) {
      console.error('Failed to delete mission:', e);
    }
  };

  const handleEdit = (m: CronMission) => {
    setEditingMission(m);
    setTab('edit');
  };

  const handleViewRuns = (m: CronMission) => {
    setViewingRunsMission(m);
    setTab('runs');
  };

  const handleSave = async (_mission: CronMission) => {
    setEditingMission(null);
    setTab('list');
    await loadMissions();
  };

  const handleCancel = () => {
    setEditingMission(null);
    setTab('list');
  };

  const enabledCount = missions.filter(m => m.enabled).length;

  const tabBar = (
    <div className={cn(
      'flex items-center gap-1 px-6 py-2',
      !embedded && 'border-b border-slate-200 dark:border-white/5 bg-white/50 dark:bg-white/[0.02]',
    )}>
      <button
        onClick={() => { setTab('list'); setEditingMission(null); setViewingRunsMission(null); }}
        className={cn(
          'px-3 py-1.5 rounded-md text-xs font-semibold transition-colors',
          tab === 'list'
            ? 'bg-blue-600 text-white'
            : 'text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-white/5',
        )}
      >
        Missions
      </button>
      {tab === 'create' && (
        <span className="px-3 py-1.5 rounded-md text-xs font-semibold bg-blue-600 text-white">
          New
        </span>
      )}
      {tab === 'edit' && (
        <span className="px-3 py-1.5 rounded-md text-xs font-semibold bg-blue-600 text-white">
          Edit
        </span>
      )}
      {tab === 'runs' && (
        <span className="px-3 py-1.5 rounded-md text-xs font-semibold bg-blue-600 text-white">
          Runs
        </span>
      )}
    </div>
  );

  const content = (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="max-w-4xl mx-auto">
        {loading ? (
          <div className="text-center py-16 text-sm text-slate-400">Loading...</div>
        ) : tab === 'list' ? (
          <MissionList
            missions={missions}
            projectRoot={projectRoot}
            onToggle={handleToggle}
            onEdit={handleEdit}
            onDelete={handleDelete}
            onViewRuns={handleViewRuns}
            onCreate={() => { setEditingMission(null); setTab('create'); }}
          />
        ) : tab === 'create' || tab === 'edit' ? (
          <MissionEditor
            editing={editingMission}
            agents={agents}
            projectRoot={projectRoot}
            onSave={handleSave}
            onCancel={handleCancel}
          />
        ) : tab === 'runs' && viewingRunsMission ? (
          <RunsView
            mission={viewingRunsMission}
            projectRoot={projectRoot}
            onBack={() => { setViewingRunsMission(null); setTab('list'); }}
          />
        ) : null}
      </div>
    </div>
  );

  if (embedded) {
    return (
      <div className="flex flex-col h-full">
        {tabBar}
        {content}
      </div>
    );
  }

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200">
      <header className="flex items-center gap-4 px-6 py-3 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md">
        <button onClick={onBack} className="p-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500 transition-colors">
          <ArrowLeft size={16} />
        </button>
        <div className="flex items-center gap-2">
          <Target size={18} className={enabledCount > 0 ? 'text-green-500' : 'text-slate-400'} />
          <h1 className="text-lg font-bold tracking-tight">Missions</h1>
        </div>
        {enabledCount > 0 && (
          <span className="text-[10px] font-bold uppercase tracking-wide px-2 py-0.5 rounded-full bg-green-500/15 text-green-600 dark:text-green-400">
            {enabledCount} active
          </span>
        )}
      </header>

      {tabBar}
      {content}
    </div>
  );
};
