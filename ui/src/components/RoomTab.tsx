import React, { useState } from 'react';
import { Check, Users, Link } from 'lucide-react';
import { useServerStore } from '../stores/serverStore';
import { CreateRoomForm } from './room/CreateRoomForm';
import { JoinedRoomsCard } from './room/JoinedRoomsCard';
import { PublicRoomsCard } from './room/PublicRoomsCard';
import { RoomConfigCard } from './room/RoomConfigCard';
import { useMyRoom, type MyRoom } from './room/useMyRoom';
import { useRoomDirectory, type RoomDirectory } from './room/useRoomDirectory';
import { btnPrimary, labelCls, sectionCls } from './room/roomStyles';

const MyRoomPanel: React.FC<{ my: MyRoom; setError: (e: string | null) => void }> = ({ my, setError }) => {
  const [creating, setCreating] = useState(false);
  // Models and skills come from the page_state push, not a refetch.
  const models = useServerStore((s) => s.models);
  const skills = useServerStore((s) => s.skills);

  if (!my.loggedIn) {
    return (
      <div className={`${sectionCls} !space-y-2 text-center`}>
        <Users size={18} className="mx-auto text-slate-400" />
        <p className="text-[10px] text-slate-500">Sign in to linggen.dev to create or manage your room.</p>
        <p className="text-[10px] text-slate-400">Click the avatar in the top bar to sign in.</p>
      </div>
    );
  }
  if (my.room) return <RoomConfigCard my={my} room={my.room} models={models} skills={skills} />;
  if (creating) {
    return (
      <CreateRoomForm
        saving={my.saving}
        onCreate={async (room) => { if (await my.createRoom(room)) setCreating(false); }}
        onCancel={() => { setCreating(false); setError(null); }}
      />
    );
  }
  return (
    <div className={`${sectionCls} !space-y-2 text-center`}>
      <Users size={18} className="mx-auto text-slate-400" />
      <p className="text-[10px] text-slate-500">Share your models with others.</p>
      <button onClick={() => setCreating(true)} className={btnPrimary}>Create Room</button>
    </div>
  );
};

const InviteInput: React.FC<{ dir: RoomDirectory }> = ({ dir }) => {
  const [input, setInput] = useState('');
  const submit = async () => {
    if (await dir.joinByInvite(input.trim())) setInput('');
  };
  return (
    <div className="flex items-center gap-2">
      <div className="relative flex-1">
        <Link size={11} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-slate-400" />
        <input
          placeholder="Paste invite link to join a private room..."
          value={input}
          onChange={e => setInput(e.target.value)}
          onKeyDown={e => { if (e.nativeEvent.isComposing || e.keyCode === 229) return; if (e.key === 'Enter') submit(); }}
          className="w-full pl-7 pr-3 py-1.5 bg-white dark:bg-[#0a0a0a] border border-slate-200 dark:border-white/10 rounded-lg text-[11px] outline-none focus:ring-1 focus:ring-blue-500/50"
        />
      </div>
      <button onClick={submit} disabled={dir.joining || !input.trim()} className={btnPrimary}>
        {dir.joining ? '...' : 'Join'}
      </button>
    </div>
  );
};

export const RoomTab: React.FC = () => {
  const [error, setError] = useState<string | null>(null);
  const [publicTick, setPublicTick] = useState(0);
  const my = useMyRoom(setError, () => setPublicTick(t => t + 1));
  const dir = useRoomDirectory(setError, my.loggedIn, publicTick);

  if (my.loading) {
    return <div className="text-center py-12 text-slate-400">Loading...</div>;
  }

  return (
    <div className="space-y-4 relative">
      {my.savedTick > 0 && (
        <div
          key={my.savedTick}
          className="fixed top-4 right-4 z-50 flex items-center gap-1.5 px-3 py-1.5 bg-green-500/15 border border-green-500/30 text-green-600 dark:text-green-400 text-xs font-bold rounded-lg shadow-lg pointer-events-none"
        >
          <Check size={12} /> Saved
        </div>
      )}

      {error && (
        <div className="p-3 bg-red-500/10 border border-red-500/20 rounded-lg">
          <p className="text-xs text-red-500">{error}</p>
        </div>
      )}

      {/* Left: my room (owner config). Right: rooms I joined + public ones. */}
      <div className="flex flex-col lg:flex-row gap-6 lg:items-start">
        <div className="lg:w-1/2 lg:shrink-0 space-y-3">
          <h3 className={labelCls}>My Room</h3>
          <MyRoomPanel my={my} setError={setError} />
        </div>

        <div className="hidden lg:block w-px self-stretch bg-slate-200 dark:bg-white/5" />

        <div className="flex-1 min-w-0 space-y-4">
          <h3 className={labelCls}>Rooms</h3>
          <InviteInput dir={dir} />
          <JoinedRoomsCard dir={dir} />
          <PublicRoomsCard dir={dir} ownRoomId={my.room?.id} />
          {dir.joinedRooms.length === 0 && dir.publicRooms.length === 0 && (
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
