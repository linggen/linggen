import React from 'react';
import { inputCls } from './kit';

// This machine's memory: the primary (default), or a replica of another
// machine's. The local ling-mem owns the setting; the engine forwards.

interface Role {
  role: 'primary' | 'replica';
  upstream: string | null;
  approved: boolean;
  waiting_for_approval: boolean;
  online: boolean;
  last_sync: string | null;
  queued: number;
  error: string | null;
}

async function api(init?: RequestInit): Promise<Role> {
  const resp = await fetch('/api/ling-mem/primary', init);
  const json = await resp.json().catch(() => null);
  if (!resp.ok || !json?.ok) throw new Error(json?.error ?? `HTTP ${resp.status}`);
  return json.data as Role;
}

function fact(r: Role): string {
  if (r.role === 'primary') return 'This machine holds the memory.';
  const state = !r.approved ? 'not approved yet' : r.online ? 'online' : 'offline';
  const last = r.last_sync ? ` · last sync ${new Date(r.last_sync).toLocaleString()}` : '';
  return `Replica of ${r.upstream} · ${state}${last} · ${r.queued} queued`;
}

export const LingMemPrimary: React.FC = () => {
  const [role, setRole] = React.useState<Role | null>(null);
  const [text, setText] = React.useState('');
  const [note, setNote] = React.useState('');

  const show = React.useCallback((r: Role) => {
    setRole(r);
    setText(r.role === 'primary' ? 'local' : r.upstream ?? '');
  }, []);

  React.useEffect(() => {
    api().then(show).catch((e) => setNote(String(e.message ?? e)));
  }, [show]);

  // While the primary has not clicked Allow yet, keep the facts fresh.
  React.useEffect(() => {
    if (!role?.waiting_for_approval) return;
    const t = setTimeout(() => api().then(setRole).catch(() => {}), 2000);
    return () => clearTimeout(t);
  }, [role]);

  const save = async () => {
    setNote('');
    try {
      show(await api({
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ primary: text.trim() || 'local' }),
      }));
    } catch (e) {
      setNote(`Not saved: ${(e as Error).message}`);
    }
  };

  const hint = role?.waiting_for_approval
    ? `Click Allow on ${role.upstream?.split(':')[0]}.`
    : role?.error ?? '';
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex gap-2">
        <input
          className={inputCls}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') save(); }}
          placeholder="local, or 192.168.1.5:9528"
        />
        <button
          className="px-3 py-1.5 rounded-lg text-xs font-bold bg-blue-600 text-white hover:bg-blue-700"
          onClick={save}
        >
          Save
        </button>
      </div>
      {role && <span className="text-xs text-slate-500 dark:text-slate-400">{fact(role)}</span>}
      {hint && <span className="text-xs text-slate-500 dark:text-slate-400">{hint}</span>}
      {note && <span className="text-xs text-slate-500 dark:text-slate-400">{note}</span>}
    </div>
  );
};
