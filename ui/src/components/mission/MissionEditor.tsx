import React, { useEffect, useState } from 'react';
import { Eye } from 'lucide-react';
import { cn } from '../../lib/cn';
import type { CronMission } from '../../types';
import { createMission, updateMission } from '../../lib/missions-api';
import { CRON_PRESETS, PERMISSION_MODES, describeCron } from '../../lib/mission-utils';

export const MissionEditor: React.FC<{
  editing: CronMission | null;
  workingFolders: string[];
  onSave: (mission: CronMission) => void;
  onCancel: () => void;
  onViewAgent: () => void;
}> = ({ editing, workingFolders, onSave, onCancel, onViewAgent }) => {
  const [name, setName] = useState(editing?.name || '');
  const [description, setDescription] = useState(editing?.description || '');
  const [schedule, setSchedule] = useState(editing?.schedule || '*/30 * * * *');
  const [prompt, setPrompt] = useState(editing?.prompt || '');
  const [model, setModel] = useState(editing?.model || '');
  const [selectedCwd, setSelectedCwd] = useState(editing?.cwd || editing?.project || '');
  const [entry, setEntry] = useState(editing?.entry || '');

  // Default mode is the most-common one across the stored per-path
  // grants (or "admin" when there are no paths yet). Saving propagates
  // it to every path in `permission_paths` via the backend's
  // `build_permission` helper.
  const initialMode = (() => {
    const grants = editing?.permission?.paths;
    if (!grants || grants.length === 0) return 'admin' as const;
    const counts = new Map<string, number>();
    for (const g of grants) counts.set(g.mode, (counts.get(g.mode) ?? 0) + 1);
    const [winner] = [...counts.entries()].sort((a, b) => b[1] - a[1]);
    const mode = winner?.[0];
    return mode === 'read' || mode === 'edit' || mode === 'admin' ? mode : 'admin';
  })();
  const [permissionMode, setPermissionMode] = useState<'read' | 'edit' | 'admin'>(initialMode);
  // Render paths as bare path strings, one per line — the per-path
  // mode is collapsed into the dropdown above. Without this map() the
  // `{path, mode}` objects render as `[object Object]`.
  const [permissionPathsText, setPermissionPathsText] = useState(
    (editing?.permission?.paths || []).map(g => g.path).join('\n'),
  );
  const [permissionWarning, setPermissionWarning] = useState(editing?.permission?.warning || '');

  const [allowSkillsText, setAllowSkillsText] = useState((editing?.allow_skills || []).join(', '));
  const [requiresText, setRequiresText] = useState((editing?.requires || []).join(', '));
  const [allowedToolsText, setAllowedToolsText] = useState((editing?.allowed_tools || []).join(', '));

  const [models, setModels] = useState<{ id: string; model: string; provider: string }[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    fetch('/api/config').then(r => r.ok ? r.json() : null).then(data => { if (data?.models) setModels(data.models); }).catch(() => {});
  }, []);

  const splitList = (s: string): string[] =>
    s.split(/[\s,]+/).map(x => x.trim()).filter(Boolean);

  const splitLines = (s: string): string[] =>
    s.split('\n').map(x => x.trim()).filter(Boolean);

  const handleSave = async () => {
    if (!schedule.trim()) { setError('Schedule is required'); return; }
    if (!prompt.trim() && !entry.trim()) {
      setError('Mission needs either a prompt body or an entry script');
      return;
    }
    setSaving(true); setError(null);
    try {
      const permPaths = splitLines(permissionPathsText);
      const payload = {
        name: name || undefined,
        description: description || '',
        schedule,
        prompt: prompt || undefined,
        model: model || undefined,
        cwd: selectedCwd || undefined,
        entry: entry || undefined,
        permission_mode: permissionMode,
        permission_paths: permPaths,
        permission_warning: permissionWarning || undefined,
        allow_skills: splitList(allowSkillsText),
        requires: splitList(requiresText),
        allowed_tools: splitList(allowedToolsText),
      };
      const result = editing
        ? await updateMission(editing.id, payload)
        : await createMission(payload);
      if (result) onSave(result);
    } catch (e: any) { setError(e.message || 'Failed to save mission'); }
    setSaving(false);
  };

  const scriptOnly = !prompt.trim() && entry.trim().length > 0;

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="max-w-2xl mx-auto space-y-4">
        <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">
          {editing ? 'Edit Mission' : 'New Mission'}
        </h2>

        {error && <div className="bg-red-500/10 border border-red-500/20 rounded-lg p-3 text-xs text-red-600 dark:text-red-400">{error}</div>}

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Name</label>
          <input type="text" value={name} onChange={e => setName(e.target.value)} placeholder="e.g. dream"
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Description</label>
          <textarea value={description} onChange={e => setDescription(e.target.value)} placeholder="One-line summary — shown in the mission list" rows={2}
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 resize-y focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Cron Schedule</label>
          <input type="text" value={schedule} onChange={e => setSchedule(e.target.value)} placeholder="*/30 * * * *"
            className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
          <div className="flex flex-wrap gap-1.5 mt-2">
            {CRON_PRESETS.map(p => (
              <button key={p.value} onClick={() => setSchedule(p.value)} className={cn(
                'text-[11px] px-2 py-0.5 rounded-full border transition-colors',
                schedule === p.value ? 'border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-400' : 'border-slate-200 dark:border-white/10 text-slate-500 hover:bg-slate-50 dark:hover:bg-white/5',
              )}>{p.label}</button>
            ))}
          </div>
          <div className="text-[11px] text-slate-400 mt-1.5">{describeCron(schedule)}</div>
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Agent</label>
          <div className="flex items-center gap-2">
            <div className="flex-1 px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-white/[0.03] text-slate-500">
              <span className="font-semibold text-purple-600 dark:text-purple-400">ling</span>
              <span className="text-slate-400 ml-2">— Autonomous (no AskUser, no UI)</span>
            </div>
            <button onClick={onViewAgent} className="flex items-center gap-1 px-2.5 py-2 text-xs font-medium rounded-lg border border-slate-200 dark:border-white/10 text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5 transition-colors shrink-0" title="View mission.md">
              <Eye size={13} /> View
            </button>
          </div>
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Entry script <span className="text-slate-400 font-normal">(optional — runs before the agent)</span>
          </label>
          <input type="text" value={entry} onChange={e => setEntry(e.target.value)}
            placeholder="scripts/collect.sh  or  inline bash -c '...'"
            className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
          <div className="text-[11px] text-slate-400 mt-1.5">
            Relative paths resolve against the mission directory. Entry receives <code>MISSION_ID</code>, <code>MISSION_DIR</code>, <code>MISSION_CWD</code>, <code>MISSION_OUTPUT_DIR</code>, <code>MISSION_LAST_RUN_AT</code>, <code>MISSION_RUN_ID</code>.
          </div>
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Permission ceiling</label>
          <div className="space-y-2">
            {PERMISSION_MODES.map(m => {
              const selected = permissionMode === m.value;
              const colorMap: Record<string, string> = {
                green: selected ? 'border-green-500/40 bg-green-500/10' : '',
                blue: selected ? 'border-blue-500/40 bg-blue-500/10' : '',
                amber: selected ? 'border-amber-500/40 bg-amber-500/10' : '',
              };
              const dotMap: Record<string, string> = {
                green: 'bg-green-500', blue: 'bg-blue-500', amber: 'bg-amber-500',
              };
              return (
                <button key={m.value} type="button" onClick={() => setPermissionMode(m.value as any)}
                  className={cn(
                    'w-full flex items-start gap-3 px-3 py-2.5 rounded-lg border text-left transition-colors',
                    selected ? colorMap[m.color] : 'border-slate-200 dark:border-white/10 hover:bg-slate-50 dark:hover:bg-white/5',
                  )}>
                  <div className={cn('w-3 h-3 rounded-full mt-0.5 shrink-0 border-2',
                    selected ? dotMap[m.color] + ' border-transparent' : 'border-slate-300 dark:border-white/20')} />
                  <div className="min-w-0">
                    <div className="text-xs font-semibold text-slate-700 dark:text-slate-200">{m.label}</div>
                    <div className="text-[11px] text-slate-500 dark:text-slate-400 mt-0.5">{m.desc}</div>
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Permission paths <span className="text-slate-400 font-normal">(one per line — extra grants beyond cwd)</span>
          </label>
          <textarea value={permissionPathsText} onChange={e => setPermissionPathsText(e.target.value)}
            placeholder={'~/.linggen/memory\n~/.claude/projects'} rows={3}
            className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 resize-y focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Permission warning <span className="text-slate-400 font-normal">(shown in UI before enabling)</span>
          </label>
          <input type="text" value={permissionWarning} onChange={e => setPermissionWarning(e.target.value)}
            placeholder="e.g. Reads session files and writes to ~/.linggen/memory"
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Allowed tools <span className="text-slate-400 font-normal">(comma-separated; empty = unrestricted)</span>
          </label>
          <input type="text" value={allowedToolsText} onChange={e => setAllowedToolsText(e.target.value)}
            placeholder="Read, Write, Edit, Bash, Glob, Grep, Task, Memory_add, Memory_search"
            className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Allow skills <span className="text-slate-400 font-normal">(via Skill tool — empty removes Skill, * allows any)</span>
          </label>
          <input type="text" value={allowSkillsText} onChange={e => setAllowSkillsText(e.target.value)}
            placeholder="memory, linggen"
            className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Requires <span className="text-slate-400 font-normal">(capabilities — validated at load)</span>
          </label>
          <input type="text" value={requiresText} onChange={e => setRequiresText(e.target.value)}
            placeholder="memory"
            className="w-full px-3 py-2 text-sm font-mono rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">Model <span className="text-slate-400">(optional)</span></label>
          <select value={model} onChange={e => setModel(e.target.value)}
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30">
            <option value="">Default (inherit from agent)</option>
            {models.map(m => <option key={m.id} value={m.id}>{m.id} — {m.provider}/{m.model}</option>)}
          </select>
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Working folder <span className="text-slate-400 font-normal">(optional, defaults to HOME)</span>
          </label>
          <select value={selectedCwd} onChange={e => setSelectedCwd(e.target.value)}
            className="w-full px-3 py-2 text-sm rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 focus:outline-none focus:ring-2 focus:ring-blue-500/30">
            <option value="">HOME (default)</option>
            {workingFolders.map(p => <option key={p} value={p}>{p.split('/').pop() || p}</option>)}
          </select>
        </div>

        <div>
          <label className="text-[12px] font-medium text-slate-600 dark:text-slate-400 mb-1.5 block">
            Prompt body {scriptOnly && <span className="text-amber-600 dark:text-amber-400">(empty — this is a script-only mission)</span>}
          </label>
          <textarea value={prompt} onChange={e => setPrompt(e.target.value)}
            placeholder="Step-by-step instructions for the agent. Leave empty for a script-only mission." rows={20}
            className="w-full px-3 py-2 text-sm font-mono leading-relaxed rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-black/20 resize-y min-h-[420px] focus:outline-none focus:ring-2 focus:ring-blue-500/30" />
        </div>

        <div className="flex items-center gap-3 pt-2">
          <button onClick={handleSave} disabled={saving || (!prompt.trim() && !entry.trim())}
            className="px-4 py-2 text-sm font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-40 disabled:cursor-not-allowed transition-colors">
            {saving ? 'Saving...' : editing ? 'Update Mission' : 'Create Mission'}
          </button>
          <button onClick={onCancel}
            className="px-4 py-2 text-sm font-semibold rounded-lg border border-slate-200 dark:border-white/10 text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5 transition-colors">
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
};
