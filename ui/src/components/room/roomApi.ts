// Room endpoints (the engine proxies linggen.dev for most of them) and
// their shapes.
import { apiDelete, apiGet, apiPatch, apiPost } from '../../lib/api';

export interface RoomData {
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

export interface Member {
  user_id: string;
  display_name: string;
  avatar_url: string | null;
  consumer_type: string;
  added_at: string;
}

export interface JoinedRoom {
  id: string;
  name: string;
  instance_id: string;
  consumer_type: string;
  owner_name: string;
  online: boolean;
  status: string;
  token_budget_daily: number | null;
}

export interface ProxyConnectionInfo {
  instance_id: string;
  room_name: string;
  owner_name: string;
  models: string[];
}

export interface PublicRoom {
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

export interface RoomConfig {
  shared_models?: string[];
  allowed_tools?: string[];
  allowed_skills?: string[];
  room_enabled?: boolean;
  token_budget_room_daily?: number | null;
}

export interface TokenUsage {
  room_total: number;
  room_budget: number | null;
}

export interface NewRoom {
  name: string;
  room_type: string;
  max_consumers: number;
  token_budget_daily: number | null;
}

/** What a join returns: enough to auto-connect the proxy. */
export interface JoinResult {
  instance_id?: string;
  owner_name?: string;
  room_name?: string;
}

export const roomApi = {
  mine: () => apiGet<{ room: RoomData | null; members?: Member[] }>('/api/rooms/mine'),
  tokenUsage: () => apiGet<TokenUsage>('/api/token-usage'),
  joined: () => apiGet<{ rooms?: JoinedRoom[] }>('/api/rooms/joined'),
  public: () => apiGet<{ rooms?: PublicRoom[] }>('/api/rooms/public'),
  proxyStatus: () => apiGet<{ connections?: ProxyConnectionInfo[] }>('/api/proxy/status'),
  config: () => apiGet<RoomConfig>('/api/room-config'),
  saveConfig: (updates: RoomConfig) => apiPost<void>('/api/room-config', updates),
  create: (room: NewRoom) => apiPost<void>('/api/rooms', room),
  // NO trailing slash: linggen.dev routes match the exact path, so
  // `/api/rooms/` misses every handler and returns an empty 200.
  update: (updates: Record<string, unknown>) => apiPatch<void>('/api/rooms', updates),
  remove: () => apiDelete<void>('/api/rooms'),
  regenerateInvite: () => apiPost<void>('/api/rooms/invite'),
  removeMember: (roomId: string, userId: string) =>
    apiDelete<void>(`/api/rooms/${roomId}/members/${userId}`),
  leave: (roomId: string) => apiDelete<void>(`/api/rooms/${roomId}/leave`),
  joinPublic: (roomId: string) =>
    apiPost<JoinResult>('/api/rooms/join-public', { room_id: roomId, consumer_type: 'linggen' }),
  joinByInvite: (token: string) =>
    apiPost<JoinResult>('/api/rooms/join', { invite_token: token, consumer_type: 'linggen' }),
  connectProxy: (instanceId: string, ownerName: string, roomName: string) =>
    apiPost<void>('/api/proxy/connect', { instance_id: instanceId, owner_name: ownerName, room_name: roomName }),
  disconnectProxy: (instanceId: string) =>
    apiPost<void>('/api/proxy/disconnect', { instance_id: instanceId }),
};
