// My room, once it exists: header, sharing (models, tools, skills), members.
import React, { useState } from 'react';
import { ExternalLink, Copy, Check, Trash2, RefreshCw, Shield, Wifi, WifiOff } from 'lucide-react';
import type { ModelInfo, SkillInfoFull } from '../../types';
import { BudgetInput } from './BudgetInput';
import type { Member, RoomData } from './roomApi';
import type { MyRoom } from './useMyRoom';
import { ALL_TOOLS, PERM_LEVEL, TOOL_PRESETS, permLevelForTools, presetForTools, skillPermLevel, skillsWithin } from './roomPerms';
import { labelCls, onlineDot, sectionCls, smallSelectCls } from './roomStyles';

const fieldLabel = 'block text-[10px] font-bold text-slate-400 mb-1';
const inviteUrl = (token: string) => `https://linggen.dev/join/${token}`;

const RoomHeader: React.FC<{ my: MyRoom; room: RoomData }> = ({ my, room }) => {
  const [copied, setCopied] = useState(false);
  const enabled = my.config.room_enabled;
  const copyInvite = () => {
    if (!room.invite_token) return;
    navigator.clipboard.writeText(inviteUrl(room.invite_token)).catch(() => {});
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };
  const usage = my.tokenUsage;
  return (
    <div className={sectionCls}>
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 min-w-0">
          <div className={onlineDot(room.online && enabled)} />
          <span className="text-sm font-bold text-slate-900 dark:text-white truncate">{room.name}</span>
          <span className={`px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wider rounded shrink-0 ${
            room.room_type === 'public' ? 'bg-blue-500/10 text-blue-500' : 'bg-slate-200 dark:bg-white/10 text-slate-500'
          }`}>{room.room_type}</span>
        </div>
        <button
          onClick={() => my.saveConfig({ room_enabled: !enabled })}
          className={`flex items-center gap-1.5 px-3 py-1.5 text-xs font-bold rounded-lg transition-colors ${
            enabled
              ? 'bg-green-500/10 text-green-600 hover:bg-green-500/20'
              : 'bg-slate-200 dark:bg-white/10 text-slate-500 hover:bg-slate-300 dark:hover:bg-white/20'
          }`}
        >
          {enabled ? <><Wifi size={12} /> Enabled</> : <><WifiOff size={12} /> Disabled</>}
        </button>
      </div>

      <div className="grid grid-cols-3 gap-3">
        <div>
          <label className={fieldLabel}>Type</label>
          <select value={room.room_type} onChange={e => my.updateRoom({ room_type: e.target.value })} className={smallSelectCls}>
            <option value="private">Private</option>
            <option value="public">Public</option>
          </select>
        </div>
        <div>
          <label className={fieldLabel}>Max</label>
          <select value={room.max_consumers} onChange={e => my.updateRoom({ max_consumers: parseInt(e.target.value) })} className={smallSelectCls}>
            {[1, 2, 3, 4].map(n => <option key={n} value={n}>{n}</option>)}
          </select>
        </div>
        <div>
          <label className={fieldLabel}>Budget</label>
          <BudgetInput
            value={usage?.room_budget ?? room.token_budget_daily}
            onChange={my.saveBudget}
            className={`${smallSelectCls} outline-none`}
          />
        </div>
      </div>

      {usage?.room_budget != null && (
        <div>
          <div className="flex items-center justify-between text-[10px] mb-0.5">
            <span className="text-slate-400">Usage (today)</span>
            <span className="font-mono text-slate-500">
              {Math.round((usage.room_total || 0) / 1000).toLocaleString()}K / {Math.round(usage.room_budget / 1000).toLocaleString()}K
            </span>
          </div>
          <div className="w-full h-1 bg-slate-200 dark:bg-white/10 rounded-full overflow-hidden">
            <div className="h-full bg-blue-500 rounded-full" style={{ width: `${Math.min(100, ((usage.room_total || 0) / usage.room_budget) * 100)}%` }} />
          </div>
        </div>
      )}

      {room.room_type === 'private' && room.invite_token && (
        <div className="flex items-center gap-1.5">
          <input readOnly value={inviteUrl(room.invite_token)} className="flex-1 px-2 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded text-xs font-mono text-slate-500 truncate" />
          <button onClick={copyInvite} className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors shrink-0">
            {copied ? <Check size={12} className="text-green-500" /> : <Copy size={12} className="text-slate-400" />}
          </button>
          <button onClick={my.regenerateInvite} disabled={my.saving} className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors shrink-0">
            <RefreshCw size={12} className="text-slate-400" />
          </button>
        </div>
      )}
    </div>
  );
};

const SharedModels: React.FC<{ my: MyRoom; models: ModelInfo[] }> = ({ my, models }) => {
  const shared = my.config.shared_models;
  // Only local models are shareable (no proxy models).
  const own = models.filter(m => !m.id.startsWith('proxy:'));
  const toggle = (id: string) =>
    my.saveConfig({ shared_models: shared.includes(id) ? shared.filter(x => x !== id) : [...shared, id] });
  return (
    <div className={sectionCls}>
      <div className="flex items-center justify-between">
        <h4 className={labelCls}>Shared Models</h4>
        <span className="text-[10px] text-slate-500">{shared.filter(id => own.some(m => m.id === id)).length}/{own.length}</span>
      </div>
      {own.length === 0 ? (
        <p className="text-[10px] text-slate-400">No models configured.</p>
      ) : (
        <div className="grid grid-cols-2 gap-x-3 gap-y-0">
          {own.map(m => (
            <label key={m.id} className="flex items-center gap-2 py-1.5 px-1 rounded hover:bg-white dark:hover:bg-white/5 cursor-pointer transition-colors min-w-0">
              <input type="checkbox" checked={shared.includes(m.id)} onChange={() => toggle(m.id)} className="accent-blue-500 w-3.5 h-3.5 shrink-0" />
              <span className="text-xs text-slate-700 dark:text-slate-300 truncate">{m.id}</span>
            </label>
          ))}
        </div>
      )}
      {shared.length === 0 && own.length > 0 && (
        <p className="text-[9px] text-amber-500">No models shared yet.</p>
      )}
    </div>
  );
};

const PRESET_LABELS: Record<string, string> = { chat: 'Chat', read: 'Read', edit: 'Edit' };

const Permissions: React.FC<{ my: MyRoom; skills: SkillInfoFull[] }> = ({ my, skills }) => {
  const tools = my.config.allowed_tools;
  // Narrowing the tools drops the skills that no longer fit.
  const saveTools = (next: string[], level: number) =>
    my.saveConfig({ allowed_tools: next, allowed_skills: skillsWithin(my.config.allowed_skills, skills, level) });
  const applyPreset = (preset: string) => saveTools(TOOL_PRESETS[preset] ?? [], PERM_LEVEL[preset] ?? 0);
  const toggleTool = (tool: string) => {
    const next = tools.includes(tool) ? tools.filter(t => t !== tool) : [...tools, tool];
    saveTools(next, permLevelForTools(next));
  };
  const active = presetForTools(tools);
  return (
    <div className={sectionCls}>
      <div className="flex items-center gap-1.5">
        <Shield size={12} className="text-slate-400" />
        <h4 className={labelCls}>Permissions</h4>
      </div>
      <div className="grid grid-cols-3 gap-1.5">
        {(['chat', 'read', 'edit'] as const).map(preset => (
          <button
            key={preset}
            onClick={() => applyPreset(preset)}
            className={`px-2 py-1.5 rounded-lg text-center text-[10px] font-bold transition-all ${
              active === preset
                ? 'border-2 border-blue-500 bg-blue-500/5 text-blue-400'
                : 'border border-slate-200 dark:border-white/10 text-slate-500 hover:border-slate-300'
            }`}
          >
            {PRESET_LABELS[preset]}
          </button>
        ))}
      </div>
      <div className="grid grid-cols-2 gap-x-2 gap-y-0 px-0.5">
        {ALL_TOOLS.map(tool => {
          const checked = tools.includes(tool);
          return (
            <label key={tool} className="flex items-center gap-1.5 py-1 cursor-pointer">
              <input type="checkbox" checked={checked} onChange={() => toggleTool(tool)} className="accent-blue-500 w-3 h-3" />
              <span className={`text-[10px] ${checked ? 'text-slate-700 dark:text-slate-300' : 'text-slate-400'}`}>{tool}</span>
            </label>
          );
        })}
      </div>
    </div>
  );
};

const SharedSkills: React.FC<{ my: MyRoom; skills: SkillInfoFull[] }> = ({ my, skills }) => {
  const allowed = my.config.allowed_skills;
  const level = permLevelForTools(my.config.allowed_tools);
  const toggle = (name: string) =>
    my.saveConfig({ allowed_skills: allowed.includes(name) ? allowed.filter(n => n !== name) : [...allowed, name] });
  if (skills.length === 0) return null;
  return (
    <div className={sectionCls}>
      <div className="flex items-center justify-between">
        <h4 className={labelCls}>Skills</h4>
        <span className="text-[10px] text-slate-500">{allowed.length}/{skills.length}</span>
      </div>
      <div className="grid grid-cols-2 gap-x-3 gap-y-0">
        {skills.map(skill => {
          const disabled = skillPermLevel(skill) > level || skill.permission?.mode === 'admin';
          return (
            <label
              key={skill.name}
              className={`flex items-center gap-2 px-1 py-1.5 rounded transition-colors min-w-0 ${
                disabled ? 'opacity-40 cursor-not-allowed' : 'hover:bg-white dark:hover:bg-white/5 cursor-pointer'
              }`}
            >
              <input
                type="checkbox"
                checked={allowed.includes(skill.name) && !disabled}
                disabled={disabled}
                onChange={() => !disabled && toggle(skill.name)}
                className="accent-blue-500 w-3.5 h-3.5 shrink-0"
              />
              <span className="text-xs text-slate-700 dark:text-slate-300 truncate">{skill.name}</span>
            </label>
          );
        })}
      </div>
    </div>
  );
};

const Members: React.FC<{ members: Member[]; max: number; onRemove: (userId: string) => void }> = ({ members, max, onRemove }) => (
  <div className={sectionCls}>
    <h4 className={labelCls}>Members ({members.length}/{max})</h4>
    {members.length === 0 ? (
      <p className="text-[10px] text-slate-400">No members yet.</p>
    ) : (
      <div className="grid grid-cols-2 gap-x-3 gap-y-0">
        {members.map(m => (
          <div key={m.user_id} className="flex items-center justify-between py-1.5 px-1 rounded hover:bg-white dark:hover:bg-white/5 transition-colors min-w-0">
            <div className="flex items-center gap-2 min-w-0">
              {m.avatar_url ? (
                <img src={m.avatar_url} alt="" className="w-5 h-5 rounded-full shrink-0 object-cover" referrerPolicy="no-referrer" />
              ) : (
                <div className="w-5 h-5 rounded-full bg-blue-500/10 text-blue-500 text-[9px] font-bold flex items-center justify-center shrink-0">
                  {(m.display_name || '?')[0].toUpperCase()}
                </div>
              )}
              <span className="text-xs text-slate-700 dark:text-slate-300 truncate">{m.display_name}</span>
            </div>
            <button onClick={() => onRemove(m.user_id)} className="p-0.5 text-slate-300 hover:text-red-500 transition-colors shrink-0">
              <Trash2 size={10} />
            </button>
          </div>
        ))}
      </div>
    )}
  </div>
);

export const RoomConfigCard: React.FC<{
  my: MyRoom;
  room: RoomData;
  models: ModelInfo[];
  skills: SkillInfoFull[];
}> = ({ my, room, models, skills }) => (
  <div className="space-y-3">
    <RoomHeader my={my} room={room} />
    <SharedModels my={my} models={models} />
    <Permissions my={my} skills={skills} />
    <SharedSkills my={my} skills={skills} />
    <Members members={my.members} max={room.max_consumers} onRemove={my.removeMember} />
    <div className="flex items-center justify-between px-1">
      <a href="https://linggen.dev/app" target="_blank" rel="noopener noreferrer" className="flex items-center gap-1 text-[10px] text-blue-500 hover:text-blue-600 font-medium">
        linggen.dev <ExternalLink size={10} />
      </a>
      <button onClick={my.deleteRoom} disabled={my.saving} className="text-[10px] text-red-500 hover:text-red-600 font-medium">
        Delete Room
      </button>
    </div>
  </div>
);
