import React from 'react';
import { Wifi, WifiOff, LogOut, Loader2 } from 'lucide-react';
import type { JoinedRoom, ProxyConnectionInfo } from './roomApi';
import type { RoomDirectory } from './useRoomDirectory';
import { statusDot } from './roomStyles';

const JoinedRoomRow: React.FC<{ jr: JoinedRoom; conn?: ProxyConnectionInfo; dir: RoomDirectory }> = ({ jr, conn, dir }) => {
  const disabled = jr.status === 'disabled';
  const connecting = dir.connectingInstance === jr.instance_id;
  return (
    <div className="px-3 py-2.5 rounded-xl border border-slate-200 dark:border-white/5 bg-white dark:bg-white/[0.02] space-y-1.5">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 min-w-0">
          <div className={statusDot(jr.online, jr.status)} />
          <span className="text-xs font-bold text-slate-900 dark:text-white truncate">{jr.name}</span>
          <span className="text-[10px] text-slate-500 shrink-0">by {jr.owner_name}</span>
          {disabled && (
            <span className="text-[9px] px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-600 dark:text-amber-400 font-bold">Disabled</span>
          )}
        </div>
        <div className="flex items-center gap-1.5 shrink-0">
          {!disabled && jr.online && !conn && (
            <button
              onClick={() => dir.connectProxyRoom(jr.instance_id, jr.owner_name, jr.name)}
              disabled={connecting}
              className="flex items-center gap-1 px-2 py-1 text-[10px] font-bold text-blue-500 bg-blue-500/10 hover:bg-blue-500/20 rounded transition-colors disabled:opacity-50"
            >
              {connecting
                ? <><Loader2 size={10} className="animate-spin" /> Connecting...</>
                : <><Wifi size={10} /> Connect</>}
            </button>
          )}
          {conn && (
            <button
              onClick={() => dir.disconnectProxyRoom(jr.instance_id)}
              className="flex items-center gap-1 px-2 py-1 text-[10px] font-bold text-slate-500 bg-slate-200/50 dark:bg-white/5 hover:bg-slate-200 dark:hover:bg-white/10 rounded transition-colors"
            >
              <WifiOff size={10} /> Disconnect
            </button>
          )}
          <button
            onClick={() => dir.leaveRoom(jr.id, jr.instance_id)}
            className="p-1 text-slate-300 hover:text-red-500 transition-colors"
            title="Leave room"
          >
            <LogOut size={12} />
          </button>
        </div>
      </div>
      {conn && conn.models.length > 0 && (
        <div className="flex items-center gap-1.5 flex-wrap">
          <span className="text-[9px] text-green-500 font-medium">Connected</span>
          {conn.models.map(m => (
            <span key={m} className="text-[9px] px-1.5 py-0.5 rounded bg-green-500/10 text-green-600 dark:text-green-400 font-mono">
              {m.replace(/^proxy:/, '')}
            </span>
          ))}
        </div>
      )}
      {disabled && <p className="text-[9px] text-amber-500">Room disabled by owner</p>}
      {!disabled && !conn && !jr.online && <p className="text-[9px] text-slate-400">Owner offline</p>}
      {!disabled && !conn && jr.online && <p className="text-[9px] text-slate-400">Click Connect to fetch models</p>}
    </div>
  );
};

export const JoinedRoomsCard: React.FC<{ dir: RoomDirectory }> = ({ dir }) => {
  if (dir.joinedRooms.length === 0) return null;
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <span className="text-[10px] font-bold uppercase tracking-wider text-slate-400">Joined</span>
        <span className="text-[9px] text-slate-400">{dir.joinedRooms.length}</span>
      </div>
      {dir.joinedRooms.map(jr => (
        <JoinedRoomRow
          key={jr.id}
          jr={jr}
          conn={dir.proxyConnections.find(c => c.instance_id === jr.instance_id)}
          dir={dir}
        />
      ))}
    </div>
  );
};
