import React from 'react';
import type { PublicRoom } from './roomApi';
import type { RoomDirectory } from './useRoomDirectory';
import { onlineDot } from './roomStyles';

const PublicRoomAction: React.FC<{ pr: PublicRoom; own: boolean; joined: boolean; dir: RoomDirectory }> = ({ pr, own, joined, dir }) => {
  if (own) return <span className="text-[9px] px-1.5 py-0.5 rounded bg-slate-200 dark:bg-white/10 text-slate-500">Yours</span>;
  if (joined) return <span className="text-[9px] px-1.5 py-0.5 rounded bg-green-500/10 text-green-500 font-medium">Joined</span>;
  const full = pr.member_count >= pr.max_consumers;
  return (
    <button
      onClick={() => dir.joinPublicRoom(pr.id)}
      disabled={dir.joining || full || !pr.online}
      className="px-2.5 py-1 bg-blue-600 hover:bg-blue-700 text-white text-[10px] font-bold rounded-md transition-colors disabled:opacity-40"
    >
      {full ? 'Full' : 'Join'}
    </button>
  );
};

export const PublicRoomsCard: React.FC<{ dir: RoomDirectory; ownRoomId?: string }> = ({ dir, ownRoomId }) => {
  const joinedIds = new Set(dir.joinedRooms.map(r => r.id));
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <span className="text-[10px] font-bold uppercase tracking-wider text-slate-400">Public</span>
        <span className="text-[9px] text-slate-400">{dir.publicRooms.length}</span>
      </div>
      {dir.publicRooms.length === 0 ? (
        <p className="text-[10px] text-slate-500 px-1">No public rooms available.</p>
      ) : (
        <div className="space-y-1">
          {dir.publicRooms.map(pr => (
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
                <PublicRoomAction pr={pr} own={ownRoomId === pr.id} joined={joinedIds.has(pr.id)} dir={dir} />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
