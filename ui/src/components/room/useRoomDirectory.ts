// The member side of rooms: rooms I joined, their proxy connections, and
// the public list.
import { useCallback, useEffect, useState } from 'react';
import { apiErrorMessage } from '../../lib/api';
import { confirmDialog } from '../../lib/confirmDialog';
import { useUserStore } from '../../stores/userStore';
import { roomApi, type JoinResult, type JoinedRoom, type ProxyConnectionInfo, type PublicRoom } from './roomApi';

const SIGN_IN_HINT = 'Click the avatar in the top bar.';

/** `publicTick`: bump it to refetch the public list (my room's visibility changed). */
export function useRoomDirectory(setError: (e: string | null) => void, loggedIn: boolean, publicTick: number) {
  const [joinedRooms, setJoinedRooms] = useState<JoinedRoom[]>([]);
  const [proxyConnections, setProxyConnections] = useState<ProxyConnectionInfo[]>([]);
  const [publicRooms, setPublicRooms] = useState<PublicRoom[]>([]);
  const [joining, setJoining] = useState(false);
  const [connectingInstance, setConnectingInstance] = useState<string | null>(null);

  const fetchJoinedRooms = useCallback(() => {
    roomApi.joined()
      .then(data => setJoinedRooms((data.rooms || []).filter(r => r.consumer_type === 'linggen')))
      .catch(() => {});
  }, []);

  const fetchProxyStatus = useCallback(() => {
    roomApi.proxyStatus().then(data => {
      const conns = data.connections || [];
      setProxyConnections(conns);
      // Header and chat card show the proxy room's name.
      useUserStore.getState().setProxyRoom(conns.length > 0 ? conns[0].room_name : null);
    }).catch(() => {});
  }, []);

  const fetchPublicRooms = useCallback(() => {
    roomApi.public().then(data => setPublicRooms(data.rooms || [])).catch(() => {});
  }, []);

  useEffect(() => {
    fetchJoinedRooms();
    fetchProxyStatus();
  }, [fetchJoinedRooms, fetchProxyStatus]);
  useEffect(() => { fetchPublicRooms(); }, [fetchPublicRooms, publicTick]);

  const connectProxyRoom = async (instanceId: string, ownerName: string, roomName: string) => {
    if (connectingInstance) return; // prevent double-click
    setError(null);
    setConnectingInstance(instanceId);
    try {
      await roomApi.connectProxy(instanceId, ownerName, roomName);
      fetchProxyStatus();
    } catch (e) {
      setError(apiErrorMessage(e, 'Failed to connect'));
    } finally { setConnectingInstance(null); }
  };

  const disconnectProxyRoom = async (instanceId: string) => {
    try {
      await roomApi.disconnectProxy(instanceId);
      fetchProxyStatus();
    } catch { /* ignore */ }
  };

  const leaveRoom = async (roomId: string, instanceId: string) => {
    if (!(await confirmDialog('Leave this room?'))) return;
    try {
      // Disconnect the proxy FIRST so no in-flight offer hits the relay
      // after membership is gone.
      if (proxyConnections.some(c => c.instance_id === instanceId)) {
        await roomApi.disconnectProxy(instanceId).catch(() => {});
      }
      await roomApi.leave(roomId);
    } catch {
      setError('Failed to leave room');
    } finally {
      fetchJoinedRooms();
      fetchProxyStatus();
      fetchPublicRooms();
    }
  };

  /** Shared tail of both joins: refresh lists now, connect in the background. */
  const join = async (call: () => Promise<JoinResult>) => {
    setJoining(true); setError(null);
    try {
      const data = await call();
      fetchJoinedRooms();
      fetchPublicRooms();
      if (data?.instance_id) connectProxyRoom(data.instance_id, data.owner_name || '', data.room_name || '');
      return true;
    } catch (e) {
      setError(apiErrorMessage(e, 'Failed to join'));
      return false;
    } finally { setJoining(false); }
  };

  const joinPublicRoom = async (roomId: string) => {
    if (!loggedIn) {
      setError(`Sign in to linggen.dev to join a public room. ${SIGN_IN_HINT}`);
      return;
    }
    if (!(await confirmDialog('Privacy Notice: The room owner can see your messages. Don\'t share sensitive information.\n\nJoin this room?'))) return;
    await join(() => roomApi.joinPublic(roomId));
  };

  /** Join by a pasted invite link or bare token. True when it worked. */
  const joinByInvite = async (input: string) => {
    if (!input) return false;
    if (!loggedIn) {
      setError(`Sign in to linggen.dev to join a room. ${SIGN_IN_HINT}`);
      return false;
    }
    const token = input.match(/\/join\/([a-z0-9_]+)/i)?.[1] ?? input;
    return join(() => roomApi.joinByInvite(token));
  };

  return {
    joinedRooms, proxyConnections, publicRooms, joining, connectingInstance,
    connectProxyRoom, disconnectProxyRoom, leaveRoom, joinPublicRoom, joinByInvite,
  };
}

export type RoomDirectory = ReturnType<typeof useRoomDirectory>;
