import React, { useCallback, useEffect, useState } from 'react';
import { ExternalLink, Copy, Check, Trash2, RefreshCw, Users, Shield, Link, Wifi, WifiOff, LogOut } from 'lucide-react';
import type { SkillInfoFull } from '../types';
import { useUserStore } from '../stores/userStore';


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

interface JoinedRoom {
  id: string;
  name: string;
  instance_id: string;
  consumer_type: string;
  owner_name: string;
  online: boolean;
  status: string;
  token_budget_daily: number | null;
}

interface ProxyConnectionInfo {
  instance_id: string;
  room_name: string;
  owner_name: string;
  models: string[];
}

interface PublicRoom {
  id: string;
  name: string;
  owner_name: string;
  owner_avatar: string | null;
  online: boolean;
  max_consumers: number;
  member_count: number;
  token_budget_daily: number | null;
  tokens_used_today: number;
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
// Styles
// ---------------------------------------------------------------------------

const sectionCls = 'bg-white dark:bg-white/[0.02] border border-slate-200 dark:border-white/5 rounded-xl p-4 space-y-3';
const labelCls = 'text-xs font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400';
const inputCls = 'w-full bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded-lg px-3 py-2 text-sm outline-none focus:ring-1 focus:ring-blue-500/50';
const btnPrimary = 'px-3 py-1.5 bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold rounded-lg transition-colors disabled:opacity-50';
const btnDanger = 'px-3 py-1.5 text-xs text-red-500 hover:text-red-600 font-medium';

const permBadge: Record<string, string> = {
  read: 'bg-blue-500/10 text-blue-500 dark:text-blue-400',
  edit: 'bg-amber-500/10 text-amber-500 dark:text-amber-400',
  admin: 'bg-red-500/10 text-red-500 dark:text-red-400',
};

const statusDot = (online: boolean, status?: string) =>
  `w-1.5 h-1.5 rounded-full ${status === 'disabled' ? 'bg-amber-500' : online ? 'bg-green-500' : 'bg-slate-400'}`;
const onlineDot = (online: boolean) => statusDot(online);

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export const RoomTab: React.FC = () => {
  // --- Owner state (existing) ---
  const [room, setRoom] = useState<RoomData | null>(null);
  const [members, setMembers] = useState<Member[]>([]);
  const [loading, setLoading] = useState(true);
  const [loggedIn, setLoggedIn] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [saving, setSaving] = useState(false);

  // Models (filter proxy models for sharing)
  const [allModels, setAllModels] = useState<{ id: string; model: string; provider?: string }[]>([]);
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

  // Room connect state
  const [roomEnabled, setRoomEnabled] = useState(true);
  const [autoConnect, setAutoConnect] = useState(true);

  // --- Consumer state (new) ---
  const [joinedRooms, setJoinedRooms] = useState<JoinedRoom[]>([]);
  const [proxyConnections, setProxyConnections] = useState<ProxyConnectionInfo[]>([]);

  // --- Discovery state (new) ---
  const [publicRooms, setPublicRooms] = useState<PublicRoom[]>([]);
  const [inviteInput, setInviteInput] = useState('');
  const [joining, setJoining] = useState(false);

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
      } else {
        setRoom(null);
        setMembers([]);
      }
      const name = data.room?.name ?? null;
      const store = useUserStore.getState();
      if (store.userRoomName !== name) {
        store.setUserInfo(store.userPermission, name, store.userTokenBudget);
      }
    } catch { /* ignore */ } finally {
      setLoading(false);
    }
  }, []);

  const fetchJoinedRooms = useCallback(async () => {
    try {
      const resp = await fetch('/api/rooms/joined');
      if (!resp.ok) return;
      const data = await resp.json();
      setJoinedRooms((data.rooms || []).filter((r: JoinedRoom) => r.consumer_type === 'linggen'));
    } catch { /* ignore */ }
  }, []);

  const fetchProxyStatus = useCallback(async () => {
    try {
      const resp = await fetch('/api/proxy/status');
      if (!resp.ok) return;
      const data = await resp.json();
      setProxyConnections(data.connections || []);
    } catch { /* ignore */ }
  }, []);

  const fetchPublicRooms = useCallback(async () => {
    try {
      const resp = await fetch('/api/rooms/public');
      if (!resp.ok) return;
      const data = await resp.json();
      setPublicRooms(data.rooms || []);
    } catch { /* ignore */ }
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
          // /api/models returns a raw array, not { models: [...] }
          setAllModels(Array.isArray(data) ? data : data.models || []);
        }
        if (configResp.ok) {
          const data = await configResp.json();
          setSharedModels(data.shared_models || []);
          setAllowedTools(data.allowed_tools || TOOL_PRESETS.read);
          setAllowedSkills(data.allowed_skills || []);
          const enabled = data.room_enabled ?? true;
          setRoomEnabled(enabled);
          setAutoConnect(data.auto_connect ?? true);
          // Sync to global store so HeaderBar shows correct status
          useUserStore.getState().setRoomEnabled(enabled);
        }
        if (skillsResp.ok) {
          const data: SkillInfoFull[] = await skillsResp.json();
          setAllSkills(data);
        }
      } catch { /* ignore */ }
    })();
    fetchJoinedRooms();
    fetchProxyStatus();
    fetchPublicRooms();
  }, [fetchJoinedRooms, fetchProxyStatus, fetchPublicRooms]);

  // -----------------------------------------------------------------------
  // Save helpers (owner)
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
    const level = permLevelForTools(next);
    const filtered = allowedSkills.filter(name => {
      const skill = allSkills.find(s => s.name === name);
      const skillLevel = PERM_LEVEL[skill?.permission?.mode ?? 'chat'] ?? 0;
      return skillLevel <= level;
    });
    setAllowedSkills(filtered);
    saveRoomConfig({ allowed_tools: next, allowed_skills: filtered });
  };

  const toggleRoomEnabled = () => {
    const next = !roomEnabled;
    setRoomEnabled(next);
    // Sync to global store so HeaderBar updates immediately
    useUserStore.getState().setRoomEnabled(next);
    saveRoomConfig({ room_enabled: next } as any);
  };

  const toggleAutoConnect = () => {
    const next = !autoConnect;
    setAutoConnect(next);
    saveRoomConfig({ auto_connect: next } as any);
  };

  const toggleSkill = (name: string) => {
    const next = allowedSkills.includes(name)
      ? allowedSkills.filter(n => n !== name)
      : [...allowedSkills, name];
    setAllowedSkills(next);
    saveRoomConfig({ allowed_skills: next });
  };

  // -----------------------------------------------------------------------
  // Room CRUD (owner)
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
  // Consumer actions (new)
  // -----------------------------------------------------------------------

  const connectProxyRoom = async (instanceId: string, ownerName: string, roomName: string) => {
    setError(null);
    try {
      const resp = await fetch('/api/proxy/connect', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ instance_id: instanceId, owner_name: ownerName, room_name: roomName }),
      });
      if (!resp.ok) {
        const data = await resp.json();
        setError(data.error || 'Failed to connect');
        return;
      }
      fetchProxyStatus();
    } catch (e: any) { setError(e.message); }
  };

  const disconnectProxyRoom = async (instanceId: string) => {
    try {
      await fetch('/api/proxy/disconnect', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ instance_id: instanceId }),
      });
      fetchProxyStatus();
    } catch { /* ignore */ }
  };

  const leaveRoom = async (roomId: string, instanceId: string) => {
    if (!confirm('Leave this room?')) return;
    try {
      // Disconnect proxy FIRST so no in-flight offers hit the relay after membership is removed
      const conn = proxyConnections.find(c => c.instance_id === instanceId);
      if (conn) {
        await fetch('/api/proxy/disconnect', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ instance_id: instanceId }),
        });
      }
      // Then remove room membership
      const resp = await fetch(`/api/rooms/${roomId}/leave`, { method: 'DELETE' });
      if (!resp.ok) {
        setError('Failed to leave room');
        return;
      }
    } catch { /* ignore */ } finally {
      fetchJoinedRooms();
      fetchProxyStatus();
      fetchPublicRooms();
    }
  };

  const joinPublicRoom = async (roomId: string) => {
    setJoining(true); setError(null);
    try {
      const resp = await fetch('/api/rooms/join-public', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ room_id: roomId, consumer_type: 'linggen' }),
      });
      const data = await resp.json();
      if (!resp.ok) {
        setError(data.error || 'Failed to join');
        return;
      }
      // Refresh room lists immediately so the UI updates before slow WebRTC connect
      fetchJoinedRooms();
      fetchPublicRooms();
      // Auto-connect in background — don't block UI
      if (data.instance_id) {
        connectProxyRoom(data.instance_id, data.owner_name || '', data.room_name || '');
      }
    } catch (e: any) { setError(e.message); } finally { setJoining(false); }
  };

  const joinByInvite = async () => {
    const input = inviteInput.trim();
    if (!input) return;
    // Extract token from URL or use as-is
    const match = input.match(/\/join\/([a-z0-9_]+)/i);
    const token = match ? match[1] : input;

    setJoining(true); setError(null);
    try {
      const resp = await fetch('/api/rooms/join', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ invite_token: token, consumer_type: 'linggen' }),
      });
      const data = await resp.json();
      if (!resp.ok) {
        setError(data.error || 'Failed to join');
        return;
      }
      setInviteInput('');
      // Refresh room lists immediately so the UI updates before slow WebRTC connect
      fetchJoinedRooms();
      fetchPublicRooms();
      // Auto-connect in background — don't block UI
      if (data.instance_id) {
        connectProxyRoom(data.instance_id, data.owner_name || '', data.room_name || '');
      }
    } catch (e: any) { setError(e.message); } finally { setJoining(false); }
  };

  // -----------------------------------------------------------------------
  // Derived state
  // -----------------------------------------------------------------------

  const activePreset = presetForTools(allowedTools);
  const currentPermLevel = permLevelForTools(allowedTools);
  // Only show local models for sharing (no proxy models)
  const ownModels = allModels.filter(m => !m.id.startsWith('proxy:'));
  const joinedRoomIds = new Set(joinedRooms.map(r => r.id));

  // -----------------------------------------------------------------------
  // Render
  // -----------------------------------------------------------------------

  if (loading) {
    return <div className="text-center py-12 text-slate-400">Loading...</div>;
  }

  if (!loggedIn) {
    return (
      <div className="text-center py-12 space-y-3">
        <p className="text-slate-500">Sign in to linggen.dev to manage rooms.</p>
        <p className="text-xs text-slate-400">Click the avatar in the top bar to sign in.</p>
      </div>
    );
  }

  return (
    <div className="space-y-4">

      {error && (
        <div className="p-3 bg-red-500/10 border border-red-500/20 rounded-lg">
          <p className="text-xs text-red-500">{error}</p>
        </div>
      )}

      {/* ═══════════════════════════════════════════════════════════════ */}
      {/* Two-panel layout                                               */}
      {/* Left: My Room (owner config)   Right: Rooms (joined + public)  */}
      {/* ═══════════════════════════════════════════════════════════════ */}
      <div className="flex flex-col lg:flex-row gap-6 lg:items-start">

        {/* ─────────────────────────────────────────────────────────── */}
        {/* LEFT PANEL: My Room                                         */}
        {/* ─────────────────────────────────────────────────────────── */}
        <div className="lg:w-1/2 lg:shrink-0 space-y-3">
          <h3 className={labelCls}>My Room</h3>

          {/* No room — compact CTA */}
          {!room && !creating && (
            <div className={`${sectionCls} !space-y-2 text-center`}>
              <Users size={18} className="mx-auto text-slate-400" />
              <p className="text-[10px] text-slate-500">Share your models with others.</p>
              <button onClick={() => setCreating(true)} className={btnPrimary}>Create Room</button>
            </div>
          )}

          {/* Create form */}
          {creating && !room && (
            <div className={sectionCls}>
              <div className="space-y-3">
                <div>
                  <label className="block text-[10px] font-bold text-slate-400 mb-1">Name</label>
                  <input value={formName} onChange={e => setFormName(e.target.value)} className={inputCls} />
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="block text-[10px] font-bold text-slate-400 mb-1">Type</label>
                    <select value={formType} onChange={e => setFormType(e.target.value)} className={inputCls}>
                      <option value="private">Private</option>
                      <option value="public">Public</option>
                    </select>
                  </div>
                  <div>
                    <label className="block text-[10px] font-bold text-slate-400 mb-1">Max Users</label>
                    <select value={formMaxConsumers} onChange={e => setFormMaxConsumers(parseInt(e.target.value))} className={inputCls}>
                      {[1, 2, 3, 4].map(n => <option key={n} value={n}>{n}</option>)}
                    </select>
                  </div>
                </div>
                <div>
                  <label className="block text-[10px] font-bold text-slate-400 mb-1">Daily Budget</label>
                  <BudgetInput value={formBudget} onChange={setFormBudget} />
                </div>
              </div>
              <div className="flex gap-2 pt-1">
                <button onClick={createRoom} disabled={saving} className={btnPrimary}>
                  {saving ? 'Creating...' : 'Create'}
                </button>
                <button onClick={() => { setCreating(false); setError(null); }} className="text-xs text-slate-500 hover:text-slate-700 dark:hover:text-slate-300">
                  Cancel
                </button>
              </div>
            </div>
          )}

          {/* Room exists — settings card */}
          {room && (
            <div className="space-y-3">

              {/* Room header card */}
              <div className={sectionCls}>
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2 min-w-0">
                    <div className={onlineDot(room.online && roomEnabled)} />
                    <span className="text-sm font-bold text-slate-900 dark:text-white truncate">{room.name}</span>
                    <span className={`px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wider rounded shrink-0 ${
                      room.room_type === 'public' ? 'bg-blue-500/10 text-blue-500' : 'bg-slate-200 dark:bg-white/10 text-slate-500'
                    }`}>{room.room_type}</span>
                  </div>
                  <button
                    onClick={toggleRoomEnabled}
                    className={`flex items-center gap-1.5 px-3 py-1.5 text-xs font-bold rounded-lg transition-colors ${
                      roomEnabled
                        ? 'bg-green-500/10 text-green-600 hover:bg-green-500/20'
                        : 'bg-slate-200 dark:bg-white/10 text-slate-500 hover:bg-slate-300 dark:hover:bg-white/20'
                    }`}
                  >
                    {roomEnabled ? <><Wifi size={12} /> Enabled</> : <><WifiOff size={12} /> Disabled</>}
                  </button>
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
                    <label className="block text-[10px] font-bold text-slate-400 mb-1">Max</label>
                    <select value={room.max_consumers} onChange={e => updateRoom({ max_consumers: parseInt(e.target.value) })} className="w-full px-2 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded text-xs">
                      {[1, 2, 3, 4].map(n => <option key={n} value={n}>{n}</option>)}
                    </select>
                  </div>
                  <div>
                    <label className="block text-[10px] font-bold text-slate-400 mb-1">Budget</label>
                    <BudgetInput
                      value={room.token_budget_daily}
                      onChange={val => { if (val !== room.token_budget_daily) updateRoom({ token_budget_daily: val }); }}
                      className="w-full px-2 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded text-xs outline-none"
                    />
                  </div>
                </div>

                {/* Token usage bar */}
                {room.token_budget_daily != null && (
                  <div>
                    <div className="flex items-center justify-between text-[10px] mb-0.5">
                      <span className="text-slate-400">Usage</span>
                      <span className="font-mono text-slate-500">
                        {Math.round((room.tokens_used_today || 0) / 1000).toLocaleString()}K / {Math.round(room.token_budget_daily / 1000).toLocaleString()}K
                      </span>
                    </div>
                    <div className="w-full h-1 bg-slate-200 dark:bg-white/10 rounded-full overflow-hidden">
                      <div className="h-full bg-blue-500 rounded-full" style={{ width: `${Math.min(100, ((room.tokens_used_today || 0) / room.token_budget_daily) * 100)}%` }} />
                    </div>
                  </div>
                )}

                {/* Invite link */}
                {room.room_type === 'private' && room.invite_token && (
                  <div className="flex items-center gap-1.5">
                    <input readOnly value={`https://linggen.dev/join/${room.invite_token}`} className="flex-1 px-2 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded text-xs font-mono text-slate-500 truncate" />
                    <button onClick={copyInvite} className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors shrink-0">
                      {copied ? <Check size={12} className="text-green-500" /> : <Copy size={12} className="text-slate-400" />}
                    </button>
                    <button onClick={regenerateInvite} disabled={saving} className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors shrink-0">
                      <RefreshCw size={12} className="text-slate-400" />
                    </button>
                  </div>
                )}
              </div>

              {/* Shared Models */}
              <div className={sectionCls}>
                <div className="flex items-center justify-between">
                  <h4 className={labelCls}>Shared Models</h4>
                  <span className="text-[10px] text-slate-500">{sharedModels.length}/{ownModels.length}</span>
                </div>
                {ownModels.length === 0 ? (
                  <p className="text-[10px] text-slate-400">No models configured.</p>
                ) : (
                  <div className="grid grid-cols-2 gap-x-3 gap-y-0">
                    {ownModels.map(m => (
                      <label key={m.id} className="flex items-center gap-2 py-1.5 px-1 rounded hover:bg-white dark:hover:bg-white/5 cursor-pointer transition-colors min-w-0">
                        <input type="checkbox" checked={sharedModels.includes(m.id)} onChange={() => toggleSharedModel(m.id)} className="accent-blue-500 w-3.5 h-3.5 shrink-0" />
                        <span className="text-xs text-slate-700 dark:text-slate-300 truncate">{m.id}</span>
                      </label>
                    ))}
                  </div>
                )}
                {sharedModels.length === 0 && ownModels.length > 0 && (
                  <p className="text-[9px] text-amber-500">No models shared yet.</p>
                )}
              </div>

              {/* Consumer Permissions */}
              <div className={sectionCls}>
                <div className="flex items-center gap-1.5">
                  <Shield size={12} className="text-slate-400" />
                  <h4 className={labelCls}>Permissions</h4>
                </div>

                {/* Tool Presets */}
                <div className="grid grid-cols-3 gap-1.5">
                  {(['chat', 'read', 'edit'] as const).map(preset => {
                    const active = activePreset === preset;
                    const labels: Record<string, string> = { chat: 'Chat', read: 'Read', edit: 'Edit' };
                    return (
                      <button
                        key={preset}
                        onClick={() => applyPreset(preset)}
                        className={`px-2 py-1.5 rounded-lg text-center text-[10px] font-bold transition-all ${
                          active
                            ? 'border-2 border-blue-500 bg-blue-500/5 text-blue-400'
                            : 'border border-slate-200 dark:border-white/10 text-slate-500 hover:border-slate-300'
                        }`}
                      >
                        {labels[preset]}
                      </button>
                    );
                  })}
                </div>

                {/* Individual Tools */}
                <div className="grid grid-cols-2 gap-x-2 gap-y-0 px-0.5">
                  {ALL_TOOLS.map(tool => {
                    const checked = allowedTools.includes(tool);
                    return (
                      <label key={tool} className="flex items-center gap-1.5 py-1 cursor-pointer">
                        <input type="checkbox" checked={checked} onChange={() => toggleTool(tool)} className="accent-blue-500 w-3 h-3" />
                        <span className={`text-[10px] ${checked ? 'text-slate-700 dark:text-slate-300' : 'text-slate-400'}`}>{tool}</span>
                      </label>
                    );
                  })}
                </div>
              </div>

              {/* Shared Skills */}
              {allSkills.length > 0 && (
                <div className={sectionCls}>
                  <div className="flex items-center justify-between">
                    <h4 className={labelCls}>Skills</h4>
                    <span className="text-[10px] text-slate-500">{allowedSkills.length}/{allSkills.length}</span>
                  </div>
                  <div className="grid grid-cols-2 gap-x-3 gap-y-0">
                    {allSkills.map(skill => {
                      const mode = skill.permission?.mode ?? null;
                      const skillLevel = PERM_LEVEL[mode ?? 'chat'] ?? 0;
                      const disabled = skillLevel > currentPermLevel || mode === 'admin';
                      const checked = allowedSkills.includes(skill.name);
                      return (
                        <label
                          key={skill.name}
                          className={`flex items-center gap-2 px-1 py-1.5 rounded transition-colors min-w-0 ${
                            disabled ? 'opacity-40 cursor-not-allowed' : 'hover:bg-white dark:hover:bg-white/5 cursor-pointer'
                          }`}
                        >
                          <input
                            type="checkbox"
                            checked={checked && !disabled}
                            disabled={disabled}
                            onChange={() => !disabled && toggleSkill(skill.name)}
                            className="accent-blue-500 w-3.5 h-3.5 shrink-0"
                          />
                          <span className="text-xs text-slate-700 dark:text-slate-300 truncate">{skill.name}</span>
                        </label>
                      );
                    })}
                  </div>
                </div>
              )}

              {/* Members */}
              <div className={sectionCls}>
                <h4 className={labelCls}>Members ({members.length}/{room.max_consumers})</h4>
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
                        <button onClick={() => removeMember(m.user_id)} className="p-0.5 text-slate-300 hover:text-red-500 transition-colors shrink-0">
                          <Trash2 size={10} />
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </div>

              {/* Footer */}
              <div className="flex items-center justify-between px-1">
                <a href="https://linggen.dev/app" target="_blank" rel="noopener noreferrer" className="flex items-center gap-1 text-[10px] text-blue-500 hover:text-blue-600 font-medium">
                  linggen.dev <ExternalLink size={10} />
                </a>
                <button onClick={deleteRoom} disabled={saving} className="text-[10px] text-red-500 hover:text-red-600 font-medium">
                  Delete Room
                </button>
              </div>
            </div>
          )}
        </div>

        {/* Vertical divider (desktop only) */}
        <div className="hidden lg:block w-px self-stretch bg-slate-200 dark:bg-white/5" />

        {/* ─────────────────────────────────────────────────────────── */}
        {/* RIGHT PANEL: Rooms (Joined + Public + Invite)               */}
        {/* ─────────────────────────────────────────────────────────── */}
        <div className="flex-1 min-w-0 space-y-4">
          <h3 className={labelCls}>Rooms</h3>

          {/* Invite link input */}
          <div className="flex items-center gap-2">
            <div className="relative flex-1">
              <Link size={11} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-slate-400" />
              <input
                placeholder="Paste invite link to join a private room..."
                value={inviteInput}
                onChange={e => setInviteInput(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter') joinByInvite(); }}
                className="w-full pl-7 pr-3 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded-lg text-[11px] outline-none focus:ring-1 focus:ring-blue-500/50"
              />
            </div>
            <button onClick={joinByInvite} disabled={joining || !inviteInput.trim()} className={btnPrimary}>
              {joining ? '...' : 'Join'}
            </button>
          </div>

          {/* Joined rooms */}
          {joinedRooms.length > 0 && (
            <div className="space-y-2">
              <div className="flex items-center gap-2">
                <span className="text-[10px] font-bold uppercase tracking-wider text-slate-400">Joined</span>
                <span className="text-[9px] text-slate-400">{joinedRooms.length}</span>
              </div>
              {joinedRooms.map(jr => {
                const conn = proxyConnections.find(c => c.instance_id === jr.instance_id);
                const isConnected = !!conn;
                return (
                  <div key={jr.id} className="px-3 py-2.5 rounded-xl border border-slate-200 dark:border-white/5 bg-white dark:bg-white/[0.02] space-y-1.5">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-2 min-w-0">
                        <div className={statusDot(jr.online, jr.status)} />
                        <span className="text-xs font-bold text-slate-900 dark:text-white truncate">{jr.name}</span>
                        <span className="text-[10px] text-slate-500 shrink-0">by {jr.owner_name}</span>
                        {jr.status === 'disabled' && (
                          <span className="text-[9px] px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-600 dark:text-amber-400 font-bold">Disabled</span>
                        )}
                      </div>
                      <div className="flex items-center gap-1.5 shrink-0">
                        {jr.status !== 'disabled' && jr.online && !isConnected && (
                          <button
                            onClick={() => connectProxyRoom(jr.instance_id, jr.owner_name, jr.name)}
                            className="flex items-center gap-1 px-2 py-1 text-[10px] font-bold text-blue-500 bg-blue-500/10 hover:bg-blue-500/20 rounded transition-colors"
                          >
                            <Wifi size={10} /> Connect
                          </button>
                        )}
                        {isConnected && (
                          <button
                            onClick={() => disconnectProxyRoom(jr.instance_id)}
                            className="flex items-center gap-1 px-2 py-1 text-[10px] font-bold text-slate-500 bg-slate-200/50 dark:bg-white/5 hover:bg-slate-200 dark:hover:bg-white/10 rounded transition-colors"
                          >
                            <WifiOff size={10} /> Disconnect
                          </button>
                        )}
                        <button
                          onClick={() => leaveRoom(jr.id, jr.instance_id)}
                          className="p-1 text-slate-300 hover:text-red-500 transition-colors"
                          title="Leave room"
                        >
                          <LogOut size={12} />
                        </button>
                      </div>
                    </div>
                    {/* Connected models */}
                    {isConnected && conn.models.length > 0 && (
                      <div className="flex items-center gap-1.5 flex-wrap">
                        <span className="text-[9px] text-green-500 font-medium">Connected</span>
                        {conn.models.map(m => (
                          <span key={m} className="text-[9px] px-1.5 py-0.5 rounded bg-green-500/10 text-green-600 dark:text-green-400 font-mono">
                            {m.replace(/^proxy:/, '')}
                          </span>
                        ))}
                      </div>
                    )}
                    {jr.status === 'disabled' && (
                      <p className="text-[9px] text-amber-500">Room disabled by owner</p>
                    )}
                    {jr.status !== 'disabled' && !isConnected && !jr.online && (
                      <p className="text-[9px] text-slate-400">Owner offline</p>
                    )}
                    {jr.status !== 'disabled' && !isConnected && jr.online && (
                      <p className="text-[9px] text-slate-400">Click Connect to fetch models</p>
                    )}
                  </div>
                );
              })}
            </div>
          )}

          {/* Public rooms */}
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <span className="text-[10px] font-bold uppercase tracking-wider text-slate-400">Public</span>
              <span className="text-[9px] text-slate-400">{publicRooms.length}</span>
            </div>
            {publicRooms.length === 0 ? (
              <p className="text-[10px] text-slate-500 px-1">No public rooms available.</p>
            ) : (
              <div className="space-y-1">
                {publicRooms.map(pr => {
                  const alreadyJoined = joinedRoomIds.has(pr.id);
                  const isOwnRoom = room?.id === pr.id;
                  const isFull = pr.member_count >= pr.max_consumers;
                  return (
                    <div key={pr.id} className="flex items-center justify-between px-3 py-2 rounded-lg border border-slate-200 dark:border-white/5 hover:bg-white/50 dark:hover:bg-white/[0.02] transition-colors">
                      <div className="flex items-center gap-2.5 min-w-0">
                        <div className={onlineDot(pr.online)} />
                        <div className="min-w-0">
                          <div className="flex items-center gap-1.5">
                            <span className="text-[11px] font-medium text-slate-900 dark:text-white truncate">{pr.name}</span>
                            <span className="text-[9px] text-slate-500 shrink-0">by {pr.owner_name}</span>
                          </div>
                          <div className="flex items-center gap-2 text-[9px] text-slate-400 mt-0.5">
                            <span>{pr.member_count}/{pr.max_consumers}</span>
                            {pr.token_budget_daily != null && (
                              <span>{Math.round(pr.token_budget_daily / 1000).toLocaleString()}K/d</span>
                            )}
                          </div>
                        </div>
                      </div>
                      <div className="shrink-0 ml-2">
                        {isOwnRoom ? (
                          <span className="text-[9px] px-1.5 py-0.5 rounded bg-slate-200 dark:bg-white/10 text-slate-500">Yours</span>
                        ) : alreadyJoined ? (
                          <span className="text-[9px] px-1.5 py-0.5 rounded bg-green-500/10 text-green-500 font-medium">Joined</span>
                        ) : (
                          <button
                            onClick={() => joinPublicRoom(pr.id)}
                            disabled={joining || isFull || !pr.online}
                            className="px-2.5 py-1 bg-blue-600 hover:bg-blue-700 text-white text-[10px] font-bold rounded-md transition-colors disabled:opacity-40"
                          >
                            {isFull ? 'Full' : 'Join'}
                          </button>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          {/* Empty state when no rooms at all */}
          {joinedRooms.length === 0 && publicRooms.length === 0 && (
            <div className="py-12 text-center">
              <Users size={28} className="mx-auto text-slate-300 dark:text-slate-600 mb-3" />
              <p className="text-sm text-slate-500">No rooms found</p>
              <p className="text-xs text-slate-400 mt-1">Create your own room or paste an invite link above.</p>
            </div>
          )}
        </div>
      </div>

    </div>
  );
};
