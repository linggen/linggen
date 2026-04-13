import React, { useCallback, useEffect, useState } from 'react';
import { ExternalLink, Copy, Check, Trash2, RefreshCw, Users, Shield } from 'lucide-react';
import type { SkillInfoFull } from '../types';


// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface RoomData {
  id: string;
  name: string;
  room_type: string;
  instance_id: string;
  invite_token: string | null;
  max_consumers: number;
  token_budget_daily: number | null;
  tokens_used_today: number;
  online: boolean;
  member_count: number;
}

interface Member {
  user_id: string;
  display_name: string;
  avatar_url: string | null;
  consumer_type: string;
  added_at: string;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const TOOL_PRESETS: Record<string, string[]> = {
  chat: [],
  read: ['WebSearch', 'WebFetch'],
  edit: ['WebSearch', 'WebFetch', 'Read', 'Write', 'Edit', 'Glob', 'Grep', 'Bash'],
};

const ALL_TOOLS = ['WebSearch', 'WebFetch', 'Read', 'Write', 'Edit', 'Glob', 'Grep', 'Bash'];

/** Permission level hierarchy: chat(0) < read(1) < edit(2) < admin(3). */
const PERM_LEVEL: Record<string, number> = { chat: 0, read: 1, edit: 2, admin: 3 };

function presetForTools(tools: string[]): string | null {
  for (const [name, preset] of Object.entries(TOOL_PRESETS)) {
    if (preset.length === tools.length && preset.every(t => tools.includes(t))) return name;
  }
  return null;
}

function permLevelForTools(tools: string[]): number {
  const preset = presetForTools(tools);
  if (preset) return PERM_LEVEL[preset] ?? 0;
  // Custom selection: derive from highest tool present
  if (tools.some(t => ['Write', 'Edit', 'Bash'].includes(t))) return PERM_LEVEL.edit;
  if (tools.some(t => ['WebSearch', 'WebFetch', 'Read', 'Glob', 'Grep'].includes(t))) return PERM_LEVEL.read;
  return PERM_LEVEL.chat;
}

// ---------------------------------------------------------------------------
// Budget input — displays in K units with comma formatting, stores raw tokens
// ---------------------------------------------------------------------------

const BudgetInput: React.FC<{
  value: number | null;
  onChange: (val: number | null) => void;
  className?: string;
}> = ({ value, onChange, className }) => {
  const kVal = value != null ? Math.round(value / 1000) : '';
  const [text, setText] = useState(String(kVal));

  useEffect(() => {
    const k = value != null ? Math.round(value / 1000) : '';
    setText(String(k));
  }, [value]);

  const display = text === '' ? '' : Number(text.replace(/,/g, '') || 0).toLocaleString();

  return (
    <div className="flex items-center gap-1.5">
      <input
        type="text"
        placeholder="500"
        value={display}
        onChange={e => {
          const raw = e.target.value.replace(/[^0-9]/g, '');
          setText(raw);
        }}
        onBlur={() => {
          const raw = parseInt(text.replace(/,/g, '') || '0');
          onChange(raw > 0 ? raw * 1000 : null);
        }}
        onKeyDown={e => {
          if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
            e.preventDefault();
            const cur = parseInt(text.replace(/,/g, '') || '0');
            const next = e.key === 'ArrowUp' ? cur + 1 : Math.max(0, cur - 1);
            setText(String(next));
            onChange(next > 0 ? next * 1000 : null);
          }
        }}
        className={className || inputCls}
      />
      <span className="text-[10px] text-slate-500 font-medium shrink-0">K</span>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Styles (match project conventions)
// ---------------------------------------------------------------------------

const sectionCls = 'bg-white dark:bg-white/[0.02] border border-slate-200 dark:border-white/5 rounded-xl p-4 space-y-3';
const labelCls = 'text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400';
const inputCls = 'w-full bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded-lg px-3 py-2 text-xs outline-none focus:ring-1 focus:ring-blue-500/50';

const permBadge: Record<string, string> = {
  read: 'bg-blue-500/10 text-blue-500 dark:text-blue-400',
  edit: 'bg-amber-500/10 text-amber-500 dark:text-amber-400',
  admin: 'bg-red-500/10 text-red-500 dark:text-red-400',
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export const SharingTab: React.FC = () => {
  const [room, setRoom] = useState<RoomData | null>(null);
  const [members, setMembers] = useState<Member[]>([]);
  const [loading, setLoading] = useState(true);
  const [loggedIn, setLoggedIn] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [saving, setSaving] = useState(false);

  // Models
  const [allModels, setAllModels] = useState<{ id: string; model: string }[]>([]);
  const [sharedModels, setSharedModels] = useState<string[]>([]);

  // Tools & skills from room config
  const [allowedTools, setAllowedTools] = useState<string[]>(TOOL_PRESETS.read);
  const [allowedSkills, setAllowedSkills] = useState<string[]>([]);
  const [allSkills, setAllSkills] = useState<SkillInfoFull[]>([]);

  // Create form
  const [creating, setCreating] = useState(false);
  const [formName, setFormName] = useState('My Room');
  const [formType, setFormType] = useState('private');
  const [formMaxConsumers, setFormMaxConsumers] = useState(4);
  const [formBudget, setFormBudget] = useState<number | null>(500000);

  // -----------------------------------------------------------------------
  // Data fetching
  // -----------------------------------------------------------------------

  const fetchRoom = useCallback(async () => {
    try {
      const resp = await fetch('/api/rooms/mine');
      if (resp.status === 401) { setLoggedIn(false); return; }
      if (!resp.ok) return;
      const data = await resp.json();
      if (data.room) {
        setRoom(data.room);
        setMembers(data.members || []);
        // Room name is now pushed via page_state from server
      } else {
        setRoom(null);
        setMembers([]);
        // Room name cleared via page_state
      }
    } catch { /* ignore */ } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { fetchRoom(); }, [fetchRoom]);

  useEffect(() => {
    (async () => {
      try {
        const [modelsResp, configResp, skillsResp] = await Promise.all([
          fetch('/api/models'),
          fetch('/api/room-config'),
          fetch('/api/skills'),
        ]);
        if (modelsResp.ok) {
          const data = await modelsResp.json();
          setAllModels(data.models || []);
        }
        if (configResp.ok) {
          const data = await configResp.json();
          setSharedModels(data.shared_models || []);
          setAllowedTools(data.allowed_tools || TOOL_PRESETS.read);
          setAllowedSkills(data.allowed_skills || []);
        }
        if (skillsResp.ok) {
          const data: SkillInfoFull[] = await skillsResp.json();
          setAllSkills(data);
        }
      } catch { /* ignore */ }
    })();
  }, []);

  // -----------------------------------------------------------------------
  // Save helpers
  // -----------------------------------------------------------------------

  const saveRoomConfig = async (updates: Partial<{ shared_models: string[]; allowed_tools: string[]; allowed_skills: string[] }>) => {
    try {
      await fetch('/api/room-config', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(updates),
      });
    } catch { /* ignore */ }
  };

  const toggleSharedModel = async (modelId: string) => {
    const next = sharedModels.includes(modelId)
      ? sharedModels.filter(id => id !== modelId)
      : [...sharedModels, modelId];
    setSharedModels(next);
    saveRoomConfig({ shared_models: next });
  };

  const applyPreset = (preset: string) => {
    const tools = TOOL_PRESETS[preset] ?? [];
    setAllowedTools(tools);
    // Remove skills that exceed new permission level
    const level = PERM_LEVEL[preset] ?? 0;
    const filtered = allowedSkills.filter(name => {
      const skill = allSkills.find(s => s.name === name);
      const skillLevel = PERM_LEVEL[skill?.permission?.mode ?? 'chat'] ?? 0;
      return skillLevel <= level;
    });
    setAllowedSkills(filtered);
    saveRoomConfig({ allowed_tools: tools, allowed_skills: filtered });
  };

  const toggleTool = (tool: string) => {
    const next = allowedTools.includes(tool)
      ? allowedTools.filter(t => t !== tool)
      : [...allowedTools, tool];
    setAllowedTools(next);
    // Remove skills that exceed new permission level
    const level = permLevelForTools(next);
    const filtered = allowedSkills.filter(name => {
      const skill = allSkills.find(s => s.name === name);
      const skillLevel = PERM_LEVEL[skill?.permission?.mode ?? 'chat'] ?? 0;
      return skillLevel <= level;
    });
    setAllowedSkills(filtered);
    saveRoomConfig({ allowed_tools: next, allowed_skills: filtered });
  };

  const toggleSkill = (name: string) => {
    const next = allowedSkills.includes(name)
      ? allowedSkills.filter(n => n !== name)
      : [...allowedSkills, name];
    setAllowedSkills(next);
    saveRoomConfig({ allowed_skills: next });
  };

  // -----------------------------------------------------------------------
  // Room CRUD
  // -----------------------------------------------------------------------

  const createRoom = async () => {
    setSaving(true); setError(null);
    try {
      const meResp = await fetch('/api/rooms/mine');
      if (meResp.status === 401) { setLoggedIn(false); return; }
      const resp = await fetch('/api/rooms/', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: formName || 'My Room',
          room_type: formType,
          max_consumers: formMaxConsumers,
          token_budget_daily: formBudget,
        }),
      });
      const data = await resp.json();
      if (!resp.ok) { setError(data.error || 'Failed to create room'); return; }
      setCreating(false);
      fetchRoom();
    } catch (e: any) { setError(e.message); } finally { setSaving(false); }
  };

  const updateRoom = async (updates: Record<string, unknown>) => {
    setSaving(true); setError(null);
    try {
      const resp = await fetch('/api/rooms/', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(updates),
      });
      if (!resp.ok) {
        const data = await resp.json();
        setError(data.error || 'Update failed');
        return;
      }
      fetchRoom();
    } catch (e: any) { setError(e.message); } finally { setSaving(false); }
  };

  const deleteRoom = async () => {
    if (!confirm('Delete your room? All members will be removed.')) return;
    setSaving(true);
    try {
      await fetch('/api/rooms/', { method: 'DELETE' });
      setRoom(null); setMembers([]);
    } catch { /* ignore */ } finally { setSaving(false); }
  };

  const regenerateInvite = async () => {
    if (!confirm('Regenerate invite link? The old link will stop working.')) return;
    setSaving(true);
    try {
      await fetch('/api/rooms/invite', { method: 'POST' });
      fetchRoom();
    } catch { /* ignore */ } finally { setSaving(false); }
  };

  const removeMember = async (userId: string) => {
    if (!confirm('Remove this member?')) return;
    try {
      await fetch(`/api/rooms/${room!.id}/members/${userId}`, { method: 'DELETE' });
      fetchRoom();
    } catch { /* ignore */ }
  };

  const copyInvite = () => {
    if (!room?.invite_token) return;
    navigator.clipboard.writeText(`https://linggen.dev/join/${room.invite_token}`).catch(() => {});
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  // -----------------------------------------------------------------------
  // Derived state
  // -----------------------------------------------------------------------

  const activePreset = presetForTools(allowedTools);
  const currentPermLevel = permLevelForTools(allowedTools);

  // -----------------------------------------------------------------------
  // Render
  // -----------------------------------------------------------------------

  if (loading) {
    return <div className="text-center py-12 text-slate-400">Loading...</div>;
  }

  if (!loggedIn) {
    return (
      <div className="text-center py-12 space-y-3">
        <p className="text-slate-500">Sign in to linggen.dev to manage your proxy room.</p>
        <p className="text-xs text-slate-400">Click the avatar in the top bar to sign in.</p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-bold text-slate-900 dark:text-white">Proxy Room</h2>
        <p className="text-xs text-slate-500 mt-1">Share your AI models with others through a proxy room.</p>
      </div>

      {error && (
        <div className="p-3 bg-red-500/10 border border-red-500/20 rounded-lg">
          <p className="text-xs text-red-500">{error}</p>
        </div>
      )}

      {/* No room — show create */}
      {!room && !creating && (
        <div className="p-6 border border-dashed border-slate-300 dark:border-white/10 rounded-xl text-center space-y-3">
          <Users size={24} className="mx-auto text-slate-400" />
          <p className="text-sm text-slate-500">No proxy room yet.</p>
          <button
            onClick={() => setCreating(true)}
            className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold rounded-lg transition-colors"
          >
            Create Room
          </button>
        </div>
      )}

      {/* Create form */}
      {creating && !room && (
        <div className={sectionCls}>
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className={labelCls}>Room Name</label>
              <input value={formName} onChange={e => setFormName(e.target.value)} className={inputCls + ' mt-1'} />
            </div>
            <div>
              <label className={labelCls}>Type</label>
              <select value={formType} onChange={e => setFormType(e.target.value)} className={inputCls + ' mt-1'}>
                <option value="private">Private (invite only)</option>
                <option value="public">Public (anyone can join)</option>
              </select>
            </div>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className={labelCls}>Max Consumers</label>
              <select value={formMaxConsumers} onChange={e => setFormMaxConsumers(parseInt(e.target.value))} className={inputCls + ' mt-1'}>
                {[1, 2, 3, 4].map(n => <option key={n} value={n}>{n}</option>)}
              </select>
            </div>
            <div>
              <label className={labelCls}>Daily Token Budget</label>
              <div className="mt-1">
                <BudgetInput value={formBudget} onChange={setFormBudget} />
              </div>
            </div>
          </div>
          <div className="flex gap-2 pt-1">
            <button onClick={createRoom} disabled={saving} className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold rounded-lg disabled:opacity-50">
              {saving ? 'Creating...' : 'Create'}
            </button>
            <button onClick={() => { setCreating(false); setError(null); }} className="px-4 py-2 text-xs text-slate-500 hover:text-slate-700 dark:hover:text-slate-300">
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Room exists — full settings */}
      {room && (
        <div className="space-y-5">

          {/* ─── Room Info ─── */}
          <div className={sectionCls}>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <h3 className="font-bold text-slate-900 dark:text-white">{room.name}</h3>
                <span className={`px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider rounded ${
                  room.room_type === 'public' ? 'bg-blue-500/10 text-blue-500' : 'bg-slate-200 dark:bg-white/10 text-slate-500'
                }`}>{room.room_type}</span>
              </div>
              <div className={`flex items-center gap-1.5 text-[11px] font-medium ${room.online ? 'text-green-500' : 'text-slate-400'}`}>
                <div className={`w-1.5 h-1.5 rounded-full ${room.online ? 'bg-green-500' : 'bg-slate-400'}`} />
                {room.online ? 'Online' : 'Offline'}
              </div>
            </div>

            <div className="grid grid-cols-3 gap-3">
              <div>
                <label className="block text-[10px] font-bold text-slate-400 mb-1">Type</label>
                <select value={room.room_type} onChange={e => updateRoom({ room_type: e.target.value })} className="w-full px-2 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded text-xs">
                  <option value="private">Private</option>
                  <option value="public">Public</option>
                </select>
              </div>
              <div>
                <label className="block text-[10px] font-bold text-slate-400 mb-1">Max Consumers</label>
                <select value={room.max_consumers} onChange={e => updateRoom({ max_consumers: parseInt(e.target.value) })} className="w-full px-2 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded text-xs">
                  {[1, 2, 3, 4].map(n => <option key={n} value={n}>{n}</option>)}
                </select>
              </div>
              <div>
                <label className="block text-[10px] font-bold text-slate-400 mb-1">Daily Budget</label>
                <BudgetInput
                  value={room.token_budget_daily}
                  onChange={val => { if (val !== room.token_budget_daily) updateRoom({ token_budget_daily: val }); }}
                />
              </div>
            </div>

            {/* Invite link */}
            {room.room_type === 'private' && room.invite_token && (
              <div>
                <label className="block text-[10px] font-bold text-slate-400 mb-1">Invite Link</label>
                <div className="flex items-center gap-2">
                  <input readOnly value={`https://linggen.dev/join/${room.invite_token}`} className="flex-1 px-2 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded text-[11px] font-mono text-slate-500" />
                  <button onClick={copyInvite} className="p-1.5 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors">
                    {copied ? <Check size={14} className="text-green-500" /> : <Copy size={14} className="text-slate-400" />}
                  </button>
                  <button onClick={regenerateInvite} disabled={saving} className="p-1.5 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors">
                    <RefreshCw size={14} className="text-slate-400" />
                  </button>
                </div>
              </div>
            )}

            {/* Token usage */}
            {room.token_budget_daily != null && (
              <div>
                <div className="flex items-center justify-between text-[10px] mb-1">
                  <span className="text-slate-400">Token usage today</span>
                  <span className="font-mono text-slate-500">
                    {(room.tokens_used_today || 0).toLocaleString()} / {room.token_budget_daily.toLocaleString()}
                  </span>
                </div>
                <div className="w-full h-1 bg-slate-200 dark:bg-white/10 rounded-full overflow-hidden">
                  <div className="h-full bg-blue-500 rounded-full" style={{ width: `${Math.min(100, ((room.tokens_used_today || 0) / room.token_budget_daily) * 100)}%` }} />
                </div>
              </div>
            )}
          </div>

          {/* ─── Shared Models ─── */}
          <div className={sectionCls}>
            <div className="flex items-center justify-between">
              <h4 className={labelCls}>Shared Models</h4>
              <span className="text-[10px] text-slate-500">{sharedModels.length} of {allModels.length} shared</span>
            </div>
            <p className="text-[10px] text-slate-400">Which models consumers can use.</p>
            {allModels.length === 0 ? (
              <p className="text-xs text-slate-400">No models configured.</p>
            ) : (
              <div className="space-y-1">
                {allModels.map(m => (
                  <label key={m.id} className="flex items-center gap-2 py-1.5 px-2 rounded-lg hover:bg-white dark:hover:bg-white/5 cursor-pointer transition-colors">
                    <input type="checkbox" checked={sharedModels.includes(m.id)} onChange={() => toggleSharedModel(m.id)} className="accent-blue-500" />
                    <span className="text-xs text-slate-700 dark:text-slate-300">{m.id}</span>
                    <span className="text-[9px] text-slate-400 font-mono">{m.model}</span>
                  </label>
                ))}
              </div>
            )}
            {sharedModels.length === 0 && allModels.length > 0 && (
              <p className="text-[10px] text-amber-500">No models shared. Consumers won't be able to use inference.</p>
            )}
          </div>

          {/* ─── Consumer Permissions ─── */}
          <div className={sectionCls}>
            <div className="flex items-center gap-2">
              <Shield size={14} className="text-slate-400" />
              <h4 className={labelCls}>Consumer Permissions</h4>
            </div>
            <p className="text-[10px] text-slate-400">What consumers can do in your room.</p>

            {/* Tool Presets */}
            <div className="space-y-2">
              <label className="text-[10px] font-bold text-slate-400">Tool Preset</label>
              <div className="grid grid-cols-3 gap-2">
                {(['chat', 'read', 'edit'] as const).map(preset => {
                  const active = activePreset === preset;
                  const labels: Record<string, [string, string]> = {
                    chat: ['Chat', 'Conversation only'],
                    read: ['Read', 'Search & browse'],
                    edit: ['Edit', 'Full coding tools'],
                  };
                  const [title, desc] = labels[preset];
                  return (
                    <button
                      key={preset}
                      onClick={() => applyPreset(preset)}
                      className={`relative px-3 py-3 rounded-lg text-center transition-all ${
                        active
                          ? 'border-2 border-blue-500 bg-blue-500/5'
                          : 'border border-slate-200 dark:border-white/10 hover:border-slate-300 dark:hover:border-white/20'
                      }`}
                    >
                      {active && (
                        <div className="absolute -top-1.5 -right-1.5 w-4 h-4 bg-blue-500 rounded-full flex items-center justify-center">
                          <Check size={10} className="text-white" />
                        </div>
                      )}
                      <div className={`text-xs font-bold ${active ? 'text-blue-400' : 'text-slate-700 dark:text-slate-300'}`}>{title}</div>
                      <div className="text-[10px] text-slate-500 mt-0.5">{desc}</div>
                    </button>
                  );
                })}
              </div>
            </div>

            {/* Individual Tools */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <label className="text-[10px] font-bold text-slate-400">Allowed Tools</label>
                <span className="text-[10px] text-slate-500">{allowedTools.length} enabled</span>
              </div>
              <div className="grid grid-cols-2 gap-x-4 gap-y-0.5 px-1">
                {ALL_TOOLS.map(tool => {
                  const checked = allowedTools.includes(tool);
                  return (
                    <label key={tool} className="flex items-center gap-2 py-1.5 cursor-pointer">
                      <input type="checkbox" checked={checked} onChange={() => toggleTool(tool)} className="accent-blue-500 w-3.5 h-3.5" />
                      <span className={`text-xs ${checked ? 'text-slate-700 dark:text-slate-300' : 'text-slate-400 dark:text-slate-500'}`}>{tool}</span>
                    </label>
                  );
                })}
              </div>
              {activePreset === null && (
                <p className="text-[10px] text-slate-500 px-1">Custom selection. Choosing a preset will reset.</p>
              )}
            </div>
          </div>

          {/* ─── Shared Skills ─── */}
          <div className={sectionCls}>
            <div className="flex items-center justify-between">
              <h4 className={labelCls}>Shared Skills</h4>
              <span className="text-[10px] text-slate-500">{allowedSkills.length} of {allSkills.length} shared</span>
            </div>
            <p className="text-[10px] text-slate-400">Skills consumers can use. Requires matching permission level.</p>

            {allSkills.length === 0 ? (
              <p className="text-xs text-slate-400">No skills installed.</p>
            ) : (
              <div className="space-y-0.5">
                {allSkills.map(skill => {
                  const mode = skill.permission?.mode ?? null;
                  const skillLevel = PERM_LEVEL[mode ?? 'chat'] ?? 0;
                  const disabled = skillLevel > currentPermLevel || mode === 'admin';
                  const checked = allowedSkills.includes(skill.name);
                  const reason = mode === 'admin' ? 'Not shareable' : disabled ? `Needs ${mode ?? 'higher'} preset` : null;

                  return (
                    <label
                      key={skill.name}
                      className={`flex items-center gap-3 px-3 py-2.5 rounded-lg transition-colors ${
                        disabled ? 'opacity-40 cursor-not-allowed' : 'hover:bg-white dark:hover:bg-white/5 cursor-pointer'
                      }`}
                    >
                      <input
                        type="checkbox"
                        checked={checked && !disabled}
                        disabled={disabled}
                        onChange={() => !disabled && toggleSkill(skill.name)}
                        className="accent-blue-500"
                      />
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-xs font-medium text-slate-700 dark:text-slate-300">{skill.name}</span>
                          {mode && (
                            <span className={`text-[10px] px-1.5 py-0.5 rounded ${permBadge[mode] ?? 'bg-slate-500/10 text-slate-400'}`}>
                              {mode}
                            </span>
                          )}
                          {!mode && (
                            <span className="text-[10px] px-1.5 py-0.5 rounded bg-slate-500/10 text-slate-400">
                              no perm
                            </span>
                          )}
                          {skill.app && (
                            <span className="text-[10px] px-1.5 py-0.5 rounded bg-indigo-500/10 text-indigo-400">app</span>
                          )}
                        </div>
                        <div className="text-[10px] text-slate-500 mt-0.5 truncate">{skill.description}</div>
                      </div>
                      {reason && (
                        <span className={`text-[10px] shrink-0 ${mode === 'admin' ? 'text-red-400' : 'text-amber-400'}`}>{reason}</span>
                      )}
                    </label>
                  );
                })}
              </div>
            )}
          </div>

          {/* ─── Members ─── */}
          <div className={sectionCls}>
            <h4 className={labelCls}>Members ({members.length}/{room.max_consumers})</h4>
            {members.length === 0 ? (
              <p className="text-xs text-slate-400">No members yet. Share your invite link to get started.</p>
            ) : (
              <div className="space-y-1">
                {members.map(m => (
                  <div key={m.user_id} className="flex items-center justify-between py-1.5 px-2 rounded-lg hover:bg-white dark:hover:bg-white/5 transition-colors">
                    <div className="flex items-center gap-2">
                      {m.avatar_url ? (
                        <img src={m.avatar_url} alt="" className="w-5 h-5 rounded-full" />
                      ) : (
                        <div className="w-5 h-5 rounded-full bg-blue-500/10 text-blue-500 text-[9px] font-bold flex items-center justify-center">
                          {(m.display_name || '?')[0].toUpperCase()}
                        </div>
                      )}
                      <span className="text-xs text-slate-700 dark:text-slate-300">{m.display_name}</span>
                      <span className="text-[9px] text-slate-400 font-mono">{m.consumer_type}</span>
                    </div>
                    <button onClick={() => removeMember(m.user_id)} className="p-1 text-slate-300 hover:text-red-500 transition-colors">
                      <Trash2 size={12} />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* ─── Footer ─── */}
          <div className="flex items-center justify-between">
            <a href="https://linggen.dev/app" target="_blank" rel="noopener noreferrer" className="flex items-center gap-1 text-xs text-blue-500 hover:text-blue-600 font-medium">
              Manage on linggen.dev <ExternalLink size={12} />
            </a>
            <button onClick={deleteRoom} disabled={saving} className="text-xs text-red-500 hover:text-red-600 font-medium">
              Delete Room
            </button>
          </div>
        </div>
      )}
    </div>
  );
};
