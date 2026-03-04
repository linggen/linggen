import React, { useState, useEffect } from 'react';
import { ArrowLeft, Target, Bot, Clock, Activity, Save, Trash2, Timer } from 'lucide-react';
import { cn } from '../lib/cn';
import type { AgentInfo, AgentRunSummary, IdlePromptEvent, MissionInfo, MissionTab } from '../types';

const formatTimestamp = (ts: number) => {
  if (!ts || ts <= 0) return '-';
  const d = new Date(ts * 1000);
  return d.toLocaleString();
};

const formatEventTime = (ts: number) => {
  if (!ts || ts <= 0) return '-';
  const d = new Date(ts);
  return d.toLocaleTimeString();
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

// --- Tab: Mission Editor ---

const EditorTab: React.FC<{
  mission: MissionInfo | null;
  missionDraft: string | null;
  onMissionDraftChange: (v: string) => void;
  onSaveMission: (text: string) => void;
  onClearMission: () => void;
}> = ({ mission, missionDraft, onMissionDraftChange, onSaveMission, onClearMission }) => {
  const [confirmClear, setConfirmClear] = useState(false);
  const [editMode, setEditMode] = useState(true);

  // null = user hasn't touched the draft yet, fall back to mission text
  const draftText = missionDraft !== null ? missionDraft : (mission?.text || '');

  return (
    <div className="space-y-4">
      {/* Status banner */}
      {mission?.active ? (
        <div className="bg-green-500/10 border border-green-500/20 rounded-lg p-3 flex items-center gap-3">
          <span className="inline-block w-2 h-2 rounded-full bg-green-500 animate-pulse shrink-0" />
          <div>
            <span className="font-semibold text-green-600 dark:text-green-400 text-sm">Mission Active</span>
            {mission.created_at > 0 && (
              <span className="text-xs text-slate-500 ml-2">since {formatTimestamp(mission.created_at)} ({timeSince(mission.created_at)})</span>
            )}
          </div>
        </div>
      ) : (
        <div className="bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg p-3 flex items-center gap-3">
          <span className="inline-block w-2 h-2 rounded-full bg-slate-400 shrink-0" />
          <span className="text-sm text-slate-500">No Active Mission</span>
        </div>
      )}

      {/* Edit / Preview toggle */}
      <div className="flex items-center gap-1">
        <button
          onClick={() => setEditMode(true)}
          className={cn(
            'px-3 py-1 rounded-md text-xs font-semibold transition-colors',
            editMode ? 'bg-blue-600 text-white' : 'text-slate-500 hover:bg-slate-100 dark:hover:bg-white/5'
          )}
        >
          Edit
        </button>
        <button
          onClick={() => setEditMode(false)}
          className={cn(
            'px-3 py-1 rounded-md text-xs font-semibold transition-colors',
            !editMode ? 'bg-blue-600 text-white' : 'text-slate-500 hover:bg-slate-100 dark:hover:bg-white/5'
          )}
        >
          Preview
        </button>
      </div>

      {/* Textarea or Preview */}
      {editMode ? (
        <textarea
          value={draftText}
          onChange={(e) => onMissionDraftChange(e.target.value)}
          placeholder="Describe your mission goal in detail. This will guide all agents during idle scheduling..."
          rows={10}
          className="w-full px-4 py-3 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 text-slate-700 dark:text-slate-300 placeholder-slate-400 resize-y focus:outline-none focus:ring-2 focus:ring-blue-500/30 font-mono"
        />
      ) : (
        <div className="w-full px-4 py-3 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 text-slate-700 dark:text-slate-300 min-h-[200px] whitespace-pre-wrap">
          {draftText || <span className="text-slate-400 italic">Nothing to preview</span>}
        </div>
      )}

      {/* Action buttons */}
      <div className="flex items-center gap-3">
        <button
          onClick={() => onSaveMission(draftText)}
          disabled={!draftText.trim()}
          className="px-4 py-2 text-sm font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
        >
          {mission?.active ? 'Update Mission' : 'Set Mission'}
        </button>
        {mission?.active && (
          <>
            {confirmClear ? (
              <div className="flex items-center gap-2">
                <span className="text-xs text-red-500">Clear mission?</span>
                <button
                  onClick={() => { onClearMission(); setConfirmClear(false); }}
                  className="px-3 py-1.5 text-xs font-semibold rounded-lg bg-red-600 text-white hover:bg-red-700"
                >
                  Confirm
                </button>
                <button
                  onClick={() => setConfirmClear(false)}
                  className="px-3 py-1.5 text-xs font-semibold rounded-lg border border-slate-200 dark:border-white/10 text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5"
                >
                  Cancel
                </button>
              </div>
            ) : (
              <button
                onClick={() => setConfirmClear(true)}
                className="px-4 py-2 text-sm font-semibold rounded-lg border border-red-200 dark:border-red-500/20 text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-500/10 transition-colors flex items-center gap-1.5"
              >
                <Trash2 size={14} /> Clear Mission
              </button>
            )}
          </>
        )}
      </div>
    </div>
  );
};

// --- Tab: Agent Config ---

const AgentConfigCard: React.FC<{
  agent: AgentInfo;
  agentStatus?: string;
  projectRoot: string;
}> = ({ agent, agentStatus, projectRoot }) => {
  const [idlePrompt, setIdlePrompt] = useState('');
  const [idleInterval, setIdleInterval] = useState('');
  const [saving, setSaving] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [isCustom, setIsCustom] = useState(false);

  useEffect(() => {
    const loadOverride = async () => {
      try {
        const url = new URL('/api/agent-override', window.location.origin);
        url.searchParams.append('project_root', projectRoot);
        url.searchParams.append('agent_id', agent.name);
        const resp = await fetch(url.toString());
        if (!resp.ok) { setLoaded(true); return; }
        const data = await resp.json();
        if (data.idle_prompt) {
          setIdlePrompt(data.idle_prompt);
          setIsCustom(true);
        }
        if (data.idle_interval_secs) {
          setIdleInterval(String(data.idle_interval_secs));
          setIsCustom(true);
        }
      } catch { /* ignore */ }
      setLoaded(true);
    };
    loadOverride();
  }, [agent.name, projectRoot]);

  const save = async () => {
    setSaving(true);
    try {
      const resp = await fetch('/api/agent-override', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: projectRoot,
          agent_id: agent.name,
          idle_prompt: idlePrompt || null,
          idle_interval_secs: idleInterval ? Number(idleInterval) : null,
        }),
      });
      if (resp.ok) {
        setIsCustom(!!(idlePrompt || idleInterval));
      }
    } catch (e) {
      console.error('Failed to save agent override:', e);
    }
    setSaving(false);
  };

  const status = agentStatus || 'idle';

  return (
    <div className="border border-slate-200 dark:border-white/10 rounded-lg p-4 bg-white dark:bg-white/[0.02]">
      <div className="flex items-center justify-between gap-3 mb-3">
        <div className="flex items-center gap-2">
          <Bot size={16} className="text-purple-500" />
          <span className="font-semibold text-sm">{agent.name}</span>
          <span className="text-xs text-slate-500 truncate max-w-xs">{agent.description}</span>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <span
            className={cn(
              'text-[10px] font-bold px-2 py-0.5 rounded-full uppercase tracking-wide',
              status === 'working' || status === 'calling_tool'
                ? 'bg-green-500/20 text-green-600'
                : status === 'thinking' || status === 'model_loading'
                  ? 'bg-blue-500/20 text-blue-600'
                  : 'bg-slate-500/20 text-slate-500'
            )}
          >
            {status}
          </span>
          <span className={cn(
            'text-[10px] font-medium px-1.5 py-0.5 rounded',
            isCustom
              ? 'bg-amber-500/10 text-amber-600 dark:text-amber-400'
              : 'bg-slate-100 dark:bg-white/5 text-slate-500'
          )}>
            {isCustom ? 'Custom' : 'Default'}
          </span>
        </div>
      </div>

      {loaded && (
        <div className="space-y-2.5">
          <div>
            <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 mb-1 block">Idle Prompt</label>
            <textarea
              value={idlePrompt}
              onChange={(e) => setIdlePrompt(e.target.value)}
              placeholder={agent.idle_prompt || 'Standing instruction when agent is idle...'}
              rows={3}
              className="w-full px-3 py-2 text-xs rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-black/20 resize-none focus:outline-none focus:ring-1 focus:ring-blue-500/50"
            />
          </div>
          <div className="flex items-center gap-3">
            <div className="flex items-center gap-2">
              <label className="text-[11px] font-medium text-slate-600 dark:text-slate-400 whitespace-nowrap">
                <Timer size={12} className="inline mr-1" />
                Interval (seconds):
              </label>
              <input
                type="number"
                min={30}
                value={idleInterval}
                onChange={(e) => setIdleInterval(e.target.value)}
                placeholder={String(agent.idle_interval_secs || 60)}
                className="w-20 px-2 py-1.5 text-xs rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-black/20 focus:outline-none focus:ring-1 focus:ring-blue-500/50"
              />
            </div>
            <button
              onClick={save}
              disabled={saving}
              className="ml-auto px-3 py-1.5 text-xs font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50 flex items-center gap-1.5 transition-colors"
            >
              <Save size={12} /> {saving ? 'Saving...' : 'Save'}
            </button>
          </div>
        </div>
      )}
    </div>
  );
};

const AgentsTab: React.FC<{
  agents: AgentInfo[];
  agentStatus: Record<string, string>;
  projectRoot: string;
}> = ({ agents, agentStatus, projectRoot }) => {
  if (agents.length === 0) {
    return (
      <div className="text-center py-16">
        <Bot size={32} className="mx-auto text-slate-300 dark:text-slate-600 mb-3" />
        <p className="text-sm text-slate-500">No agents configured</p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
        Agent Idle Configuration ({agents.length})
      </h2>
      {agents.map((agent) => (
        <AgentConfigCard
          key={agent.name}
          agent={agent}
          agentStatus={agentStatus[agent.name] || agentStatus[agent.name.toLowerCase()]}
          projectRoot={projectRoot}
        />
      ))}
    </div>
  );
};

// --- Tab: History ---

const MissionHistoryCard: React.FC<{ mission: MissionInfo }> = ({ mission }) => (
  <div className={cn(
    'border rounded-lg p-4',
    mission.active
      ? 'border-green-500/20 bg-green-50/50 dark:bg-green-500/5'
      : 'border-slate-200 dark:border-white/10 bg-white dark:bg-white/[0.02]'
  )}>
    <div className="flex items-center gap-2 mb-3">
      <span className={cn(
        'text-[10px] font-bold px-2 py-0.5 rounded-full uppercase tracking-wide',
        mission.active
          ? 'bg-green-500/20 text-green-600 dark:text-green-400'
          : 'bg-slate-500/20 text-slate-500'
      )}>
        {mission.active ? 'Active' : 'Cleared'}
      </span>
      {mission.created_at > 0 && (
        <span className="text-xs text-slate-500">{formatTimestamp(mission.created_at)}</span>
      )}
    </div>
    <div className="text-sm text-slate-700 dark:text-slate-300 whitespace-pre-wrap leading-relaxed">
      {mission.text}
    </div>
    {mission.agents && mission.agents.length > 0 && (
      <div className="mt-3 pt-3 border-t border-slate-200/50 dark:border-white/5">
        <span className="text-[11px] font-medium text-slate-500">Participating agents:</span>
        <div className="flex flex-wrap gap-1.5 mt-1">
          {mission.agents.map((a) => (
            <span key={a.id} className="text-[10px] px-2 py-0.5 rounded-full bg-blue-500/10 text-blue-600 dark:text-blue-400 font-medium">
              {a.id}
            </span>
          ))}
        </div>
      </div>
    )}
  </div>
);

const HistoryTab: React.FC<{
  projectRoot: string;
}> = ({ projectRoot }) => {
  const [missions, setMissions] = useState<MissionInfo[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const load = async () => {
      try {
        const url = new URL('/api/missions', window.location.origin);
        url.searchParams.append('project_root', projectRoot);
        const resp = await fetch(url.toString());
        if (!resp.ok) return;
        const data = await resp.json();
        setMissions(Array.isArray(data.missions) ? data.missions : []);
      } catch { /* ignore */ }
      setLoading(false);
    };
    load();
  }, [projectRoot]);

  if (loading) {
    return <div className="text-center py-16 text-sm text-slate-400">Loading...</div>;
  }

  if (missions.length === 0) {
    return (
      <div className="text-center py-16">
        <Clock size={32} className="mx-auto text-slate-300 dark:text-slate-600 mb-3" />
        <p className="text-sm text-slate-500">No missions yet</p>
        <p className="text-[11px] text-slate-400 mt-1">Set a mission in the Editor tab to get started</p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
        Mission History ({missions.length})
      </h2>
      {missions.map((m, i) => (
        <MissionHistoryCard key={`${m.created_at}-${i}`} mission={m} />
      ))}
    </div>
  );
};

// --- Tab: Activity ---

const ActivityTab: React.FC<{
  idlePromptEvents: IdlePromptEvent[];
  agents: AgentInfo[];
  agentStatus: Record<string, string>;
  agentRunSummary: Record<string, AgentRunSummary>;
  mission: MissionInfo | null;
}> = ({ idlePromptEvents, agents, agentStatus, agentRunSummary, mission }) => {
  const participatingAgents = mission?.agents?.map((a) => a.id) || agents.map((a) => a.name);

  return (
    <div className="space-y-6">
      {/* Mission summary */}
      {mission?.active && (
        <div className="bg-slate-50 dark:bg-white/[0.02] border border-slate-200 dark:border-white/10 rounded-lg p-4">
          <h3 className="text-xs font-bold uppercase tracking-wide text-slate-500 mb-2">Mission Summary</h3>
          <div className="grid grid-cols-3 gap-4 text-center">
            <div>
              <div className="text-lg font-bold text-slate-700 dark:text-slate-300">
                {mission.created_at > 0 ? timeSince(mission.created_at) : '-'}
              </div>
              <div className="text-[10px] text-slate-500">Active Since</div>
            </div>
            <div>
              <div className="text-lg font-bold text-slate-700 dark:text-slate-300">
                {idlePromptEvents.length}
              </div>
              <div className="text-[10px] text-slate-500">Idle Triggers</div>
            </div>
            <div>
              <div className="text-lg font-bold text-slate-700 dark:text-slate-300">
                {participatingAgents.length}
              </div>
              <div className="text-[10px] text-slate-500">Agents</div>
            </div>
          </div>
        </div>
      )}

      {/* Agent status grid */}
      <div>
        <h3 className="text-xs font-bold uppercase tracking-wide text-slate-500 mb-2">Agent Status</h3>
        <div className="grid grid-cols-2 gap-2">
          {agents.map((agent) => {
            const status = agentStatus[agent.name] || agentStatus[agent.name.toLowerCase()] || 'idle';
            const run = agentRunSummary[agent.name.toLowerCase()];
            const lastEvent = idlePromptEvents.find((e) => e.agent_id === agent.name || e.agent_id === agent.name.toLowerCase());
            return (
              <div key={agent.name} className="border border-slate-200 dark:border-white/10 rounded-lg p-3 bg-white dark:bg-white/[0.02]">
                <div className="flex items-center gap-2 mb-1">
                  <Bot size={12} className="text-purple-500" />
                  <span className="font-semibold text-xs">{agent.name}</span>
                  <span
                    className={cn(
                      'ml-auto text-[9px] font-bold px-1.5 py-0.5 rounded-full uppercase',
                      status === 'working' || status === 'calling_tool'
                        ? 'bg-green-500/20 text-green-600'
                        : status === 'thinking' || status === 'model_loading'
                          ? 'bg-blue-500/20 text-blue-600'
                          : 'bg-slate-500/20 text-slate-500'
                    )}
                  >
                    {status}
                  </span>
                </div>
                <div className="text-[10px] text-slate-500">
                  {run ? `Last run: ${run.status}` : 'No runs'}
                  {lastEvent && (
                    <span className="ml-2">Last idle trigger: {formatEventTime(lastEvent.timestamp)}</span>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* Activity feed */}
      <div>
        <h3 className="text-xs font-bold uppercase tracking-wide text-slate-500 mb-2">
          <Activity size={12} className="inline mr-1" />
          Idle Prompt Activity ({idlePromptEvents.length})
        </h3>
        {idlePromptEvents.length === 0 ? (
          <div className="text-center py-8 text-sm text-slate-400">
            No idle prompt triggers yet. Events appear here when agents are triggered by the idle scheduler.
          </div>
        ) : (
          <div className="space-y-1.5 max-h-[50vh] overflow-y-auto">
            {idlePromptEvents.map((evt, i) => (
              <div key={`${evt.agent_id}-${evt.timestamp}-${i}`} className="flex items-center gap-3 px-3 py-2 rounded-lg bg-white dark:bg-white/[0.02] border border-slate-200 dark:border-white/10">
                <span className="text-[10px] text-slate-400 font-mono shrink-0">
                  {formatEventTime(evt.timestamp)}
                </span>
                <span className="text-xs font-semibold text-purple-600 dark:text-purple-400 shrink-0">
                  {evt.agent_id}
                </span>
                <span className="text-[10px] text-amber-600 dark:text-amber-400 font-medium">
                  idle_prompt_triggered
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};

// --- Main Page ---

export const MissionPage: React.FC<{
  onBack: () => void;
  projectRoot: string;
  agents: AgentInfo[];
  agentStatus: Record<string, string>;
  agentRunSummary: Record<string, AgentRunSummary>;
  mission: MissionInfo | null;
  missionDraft: string | null;
  onMissionDraftChange: (v: string) => void;
  onSaveMission: (text: string) => void;
  onClearMission: () => void;
  idlePromptEvents: IdlePromptEvent[];
  embedded?: boolean;
}> = ({
  onBack,
  projectRoot,
  agents,
  agentStatus,
  agentRunSummary,
  mission,
  missionDraft,
  onMissionDraftChange,
  onSaveMission,
  onClearMission,
  idlePromptEvents,
  embedded,
}) => {
  const [tab, setTab] = useState<MissionTab>('editor');

  const tabs: { id: MissionTab; label: string }[] = [
    { id: 'editor', label: 'Editor' },
    { id: 'agents', label: 'Agent Config' },
    { id: 'history', label: 'History' },
    { id: 'activity', label: 'Activity' },
  ];

  const tabBar = (
    <div className={cn(
      'flex items-center gap-1 px-6 py-2',
      !embedded && 'border-b border-slate-200 dark:border-white/5 bg-white/50 dark:bg-white/[0.02]',
    )}>
      {tabs.map((t) => (
        <button
          key={t.id}
          onClick={() => setTab(t.id)}
          className={cn(
            'px-3 py-1.5 rounded-md text-xs font-semibold transition-colors',
            tab === t.id
              ? 'bg-blue-600 text-white'
              : 'text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-white/5'
          )}
        >
          {t.label}
        </button>
      ))}
    </div>
  );

  const content = (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="max-w-4xl mx-auto">
        {tab === 'editor' && (
          <EditorTab
            mission={mission}
            missionDraft={missionDraft}
            onMissionDraftChange={onMissionDraftChange}
            onSaveMission={onSaveMission}
            onClearMission={onClearMission}
          />
        )}
        {tab === 'agents' && (
          <AgentsTab
            agents={agents}
            agentStatus={agentStatus}
            projectRoot={projectRoot}
          />
        )}
        {tab === 'history' && (
          <HistoryTab projectRoot={projectRoot} />
        )}
        {tab === 'activity' && (
          <ActivityTab
            idlePromptEvents={idlePromptEvents}
            agents={agents}
            agentStatus={agentStatus}
            agentRunSummary={agentRunSummary}
            mission={mission}
          />
        )}
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
      {/* Header */}
      <header className="flex items-center gap-4 px-6 py-3 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md">
        <button onClick={onBack} className="p-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500 transition-colors">
          <ArrowLeft size={16} />
        </button>
        <div className="flex items-center gap-2">
          <Target size={18} className={mission?.active ? 'text-green-500' : 'text-slate-400'} />
          <h1 className="text-lg font-bold tracking-tight">Mission</h1>
        </div>
        {mission?.active && (
          <span className="text-[10px] font-bold uppercase tracking-wide px-2 py-0.5 rounded-full bg-green-500/15 text-green-600 dark:text-green-400">
            Active
          </span>
        )}
      </header>

      {tabBar}
      {content}
    </div>
  );
};
