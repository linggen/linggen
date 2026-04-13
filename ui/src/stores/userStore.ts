/**
 * User identity, permission, and connection state.
 */
import { create } from 'zustand';

interface UserState {
  userPermission: 'admin' | 'edit' | 'read' | 'chat' | 'pending';
  userRoomName: string | null;
  userTokenBudget: number | null;
  connectionStatus: 'connected' | 'reconnecting' | 'disconnected';

  setUserInfo: (permission: string, roomName?: string | null, tokenBudget?: number | null) => void;
  setConnectionStatus: (status: 'connected' | 'reconnecting' | 'disconnected') => void;
}

const isRemote = typeof document !== 'undefined' && !!document.querySelector('meta[name="linggen-instance"]');

export const useUserStore = create<UserState>((set) => ({
  userPermission: isRemote ? 'pending' as any : 'admin',
  userRoomName: null,
  userTokenBudget: null,
  connectionStatus: isRemote ? 'disconnected' : 'connected',

  setUserInfo: (permission, roomName, tokenBudget) => set({
    userPermission: permission as any,
    userRoomName: roomName ?? null,
    userTokenBudget: tokenBudget ?? null,
  }),
  setConnectionStatus: (status) => set({ connectionStatus: status }),
}));
