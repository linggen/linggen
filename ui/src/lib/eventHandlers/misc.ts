import type { UiEvent } from '../../types';
import { useSessionStore } from '../../stores/sessionStore';
import { useUiStore } from '../../stores/uiStore';
import { useUserStore } from '../../stores/userStore';
import { useRoomChatStore } from '../../stores/roomChatStore';

// ---------------------------------------------------------------------------
// Notification — mission completed, session created, etc.
// ---------------------------------------------------------------------------

export function handleNotification(item: UiEvent): void {
  const data = item.data;
  if (!data) return;

  switch (data.kind as string) {
    case 'mission_completed': {
      const name = String(data.mission_name || data.mission_id || 'Mission');
      const status = String(data.status || 'completed');
      const variant = status === 'completed' ? 'success' as const : 'error' as const;
      const label = status === 'completed' ? 'completed' : 'failed';
      useUiStore.getState().addToast({ message: `Mission "${name}" ${label}`, variant });
      useUiStore.getState().bumpMissionRefreshKey();
      return;
    }
    case 'session_created': {
      // page_state push will deliver the updated session list within 2s.
      // No HTTP fetch needed.
      return;
    }
  }
}

// ---------------------------------------------------------------------------
// App launched — skill fired an external/popup app
// ---------------------------------------------------------------------------

export function handleAppLaunched(item: UiEvent): void {
  const url = item.data?.url || '';
  if (!url) return;
  const instanceMeta = document.querySelector('meta[name="linggen-instance"]');
  if (!instanceMeta) {
    window.open(url, '_blank');
    return;
  }
  const instanceId = instanceMeta.getAttribute('content') || '';
  const relayOrigin = document.querySelector('meta[name="linggen-relay-origin"]')?.getAttribute('content') || '';
  window.open(`${relayOrigin}/app/connect/${instanceId}?app=${encodeURIComponent(url)}`, '_blank');
}

// ---------------------------------------------------------------------------
// Working folder — session's cwd/project changed
// ---------------------------------------------------------------------------

export function handleWorkingFolder(item: UiEvent): void {
  const data = item.data;
  if (!data || !item.session_id) return;

  const store = useSessionStore.getState();
  const sessions = store.allSessions.map((s) => {
    if (s.id !== item.session_id) return s;
    return {
      ...s,
      cwd: data.cwd as string,
      project: data.project as string | undefined,
      project_name: data.project_name as string | undefined,
    };
  });
  useSessionStore.setState({ allSessions: sessions });

  // If this is the active session, update the global project root so
  // API calls, file tree, and sidebar reflect the new working folder.
  if (item.session_id !== store.activeSessionId) return;
  const newRoot = (data.project as string) || (data.cwd as string);
  if (newRoot && newRoot !== store.selectedProjectRoot) {
    store.setSelectedProjectRoot(newRoot);
  }
}

// ---------------------------------------------------------------------------
// User info — sent on control channel open for ALL peers
// ---------------------------------------------------------------------------

export function handleUserInfo(item: UiEvent): void {
  const data = item.data;
  if (!data) return;

  const userStore = useUserStore.getState();

  // Structured format: { user: { user_id, user_type, permission? }, room?: { room_name, ... } }
  const user = data.user || data;
  const room = data.room;

  const userType = user.user_type || 'owner';
  if (user.user_id) userStore.setUserId(user.user_id);
  userStore.setUserProfile(user.user_name || null, user.avatar_url || null);
  userStore.setUserType(userType as 'owner' | 'consumer');

  const perm = userType === 'consumer' ? (room?.permission || 'read') : 'admin';
  const roomName = room?.room_name ?? null;
  const tokenBudget = room?.token_budget_daily ?? null;
  userStore.setUserInfo(perm, roomName, tokenBudget);

  useUiStore.getState().setCurrentPage(userType === 'consumer' ? 'consumer' : 'main');
}

// ---------------------------------------------------------------------------
// Room chat — relayed between all peers in a proxy room
// ---------------------------------------------------------------------------

export function handleRoomChat(item: UiEvent): void {
  const data = item.data;
  if (!data?.text) return;
  const senderId = data.sender_id || '';
  const localUserId = useUserStore.getState().userId || '';
  useRoomChatStore.getState().addMessage({
    senderId,
    senderName: data.sender_name || 'Unknown',
    avatarUrl: data.avatar_url || null,
    text: data.text,
    timestamp: item.ts_ms || Date.now(),
    isMine: senderId !== '' && senderId === localUserId,
  });
}

// ---------------------------------------------------------------------------
// Device topics — control messages relayed between the user's own devices
// (phone ↔ Mac: DJ library changes, media review actions). The console is a
// bystander: it keeps the wire honest by handling the kind, but the apps act.
// ---------------------------------------------------------------------------

export function handleDeviceTopic(_item: UiEvent): void {}
