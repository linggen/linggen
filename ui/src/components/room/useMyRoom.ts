// The owner side of rooms: my room, its members, its sharing config.
import { useCallback, useEffect, useState } from 'react';
import { ApiError, apiErrorMessage } from '../../lib/api';
import { confirmDialog } from '../../lib/confirmDialog';
import { useUserStore } from '../../stores/userStore';
import { roomApi, type Member, type NewRoom, type RoomConfig, type RoomData, type TokenUsage } from './roomApi';
import { TOOL_PRESETS } from './roomPerms';

const isUnauthorized = (e: unknown) => e instanceof ApiError && e.status === 401;

/** Mirror the room name into the user store (header + chat card read it). */
function syncRoomName(name: string | null) {
  const store = useUserStore.getState();
  if (store.userRoomName !== name) store.setUserInfo(store.userPermission, name, store.userTokenBudget);
}

export function useMyRoom(setError: (e: string | null) => void, onVisibilityChange: () => void) {
  const [room, setRoom] = useState<RoomData | null>(null);
  const [members, setMembers] = useState<Member[]>([]);
  const [loading, setLoading] = useState(true);
  const [loggedIn, setLoggedIn] = useState(true);
  const [saving, setSaving] = useState(false);
  const [savedTick, setSavedTick] = useState(0);
  const [tokenUsage, setTokenUsage] = useState<TokenUsage | null>(null);
  const [config, setConfig] = useState<Required<Pick<RoomConfig, 'shared_models' | 'allowed_tools' | 'allowed_skills' | 'room_enabled'>>>({
    shared_models: [],
    allowed_tools: TOOL_PRESETS.read,
    allowed_skills: [],
    room_enabled: true,
  });
  const flashSaved = () => setSavedTick(t => t + 1);

  useEffect(() => {
    if (!savedTick) return;
    const id = setTimeout(() => setSavedTick(0), 1800);
    return () => clearTimeout(id);
  }, [savedTick]);

  const fetchRoom = useCallback(async () => {
    try {
      const data = await roomApi.mine();
      setRoom(data.room ?? null);
      setMembers(data.room ? data.members || [] : []);
      syncRoomName(data.room?.name ?? null);
      roomApi.tokenUsage().then(setTokenUsage).catch(() => {});
    } catch (e) {
      if (isUnauthorized(e)) setLoggedIn(false);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { fetchRoom(); }, [fetchRoom]);

  useEffect(() => {
    roomApi.config().then((data) => {
      const enabled = data.room_enabled ?? true;
      setConfig({
        shared_models: data.shared_models || [],
        allowed_tools: data.allowed_tools || TOOL_PRESETS.read,
        allowed_skills: data.allowed_skills || [],
        room_enabled: enabled,
      });
      // Sync to global store so HeaderBar shows correct status
      useUserStore.getState().setRoomEnabled(enabled);
    }).catch(() => {});
  }, []);

  /** Apply a config change locally, then persist it. */
  const saveConfig = (updates: RoomConfig) => {
    setConfig(c => ({ ...c, ...updates }) as typeof c);
    if (updates.room_enabled !== undefined) useUserStore.getState().setRoomEnabled(updates.room_enabled);
    roomApi.saveConfig(updates).then(flashSaved).catch(() => {});
  };

  const saveBudget = (val: number | null) => {
    roomApi.saveConfig({ token_budget_room_daily: val })
      .then(flashSaved)
      .catch(() => {})
      .finally(fetchRoom);
  };

  const createRoom = async (form: NewRoom) => {
    setSaving(true); setError(null);
    try {
      await roomApi.create(form);
      await fetchRoom();
      return true;
    } catch (e) {
      if (isUnauthorized(e)) setLoggedIn(false);
      else setError(apiErrorMessage(e, 'Failed to create room'));
      return false;
    } finally { setSaving(false); }
  };

  const updateRoom = async (updates: Record<string, unknown>) => {
    setSaving(true); setError(null);
    // Optimistic, so a controlled <select> shows the new value instead of
    // snapping back during the round-trip.
    const prev = room;
    if (room) setRoom({ ...room, ...(updates as Partial<RoomData>) });
    try {
      await roomApi.update(updates);
      flashSaved();
      fetchRoom();
      // Visibility flipped: the public list gains or loses this room.
      if ('room_type' in updates) onVisibilityChange();
    } catch (e) {
      setError(apiErrorMessage(e, 'Update failed'));
      if (prev) setRoom(prev);
    } finally { setSaving(false); }
  };

  const withConfirm = (message: string, action: () => Promise<unknown>) => async () => {
    if (!(await confirmDialog(message))) return;
    setSaving(true);
    try { await action(); } catch { /* ignore */ } finally { setSaving(false); }
  };

  const deleteRoom = withConfirm('Delete your room? All members will be removed.', async () => {
    await roomApi.remove();
    setRoom(null); setMembers([]);
  });

  const regenerateInvite = withConfirm('Regenerate invite link? The old link will stop working.', async () => {
    await roomApi.regenerateInvite();
    fetchRoom();
  });

  const removeMember = async (userId: string) => {
    if (!room || !(await confirmDialog('Remove this member?'))) return;
    try {
      await roomApi.removeMember(room.id, userId);
      fetchRoom();
    } catch { /* ignore */ }
  };

  return {
    room, members, loading, loggedIn, saving, savedTick, tokenUsage, config,
    saveConfig, saveBudget, createRoom, updateRoom, deleteRoom, regenerateInvite, removeMember,
  };
}

export type MyRoom = ReturnType<typeof useMyRoom>;
