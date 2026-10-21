import React from 'react';
import { inputCls } from './kit';

// Where this machine's memory lives — `~/.linggen/client.json`, the file
// `/linggen:config` writes too. The state line is a fact from a live probe.

interface ClientView {
  ling_mem: string;
  local: boolean;
  has_token: boolean;
  state: 'ok' | 'not_approved' | 'no_answer' | 'error';
}

const STATE_TEXT: Record<ClientView['state'], string> = {
  ok: 'Connected.',
  not_approved: 'Not approved yet.',
  no_answer: 'No answer at this address.',
  error: 'Unexpected answer from this address.',
};

async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const resp = await fetch(`/api/ling-mem/client${path}`, init);
  const json = await resp.json().catch(() => null);
  if (!resp.ok || !json?.ok) throw new Error(json?.error ?? `HTTP ${resp.status}`);
  return json.data as T;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export const LingMemAddress: React.FC = () => {
  const [view, setView] = React.useState<ClientView | null>(null);
  const [text, setText] = React.useState('');
  const [note, setNote] = React.useState('');
  const [asking, setAsking] = React.useState(false);

  const show = (v: ClientView) => {
    setView(v);
    setText(v.ling_mem);
  };

  React.useEffect(() => {
    api<ClientView>('').then(show).catch((e) => setNote(String(e.message ?? e)));
  }, []);

  const save = async () => {
    if (!view || text.trim() === view.ling_mem) return;
    setNote('');
    try {
      show(await api<ClientView>('', {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ ling_mem: text }),
      }));
    } catch (e) {
      setNote(`Not saved: ${(e as Error).message}`);
    }
  };

  const requestAccess = async () => {
    setAsking(true);
    setNote('Approve this machine on the other one…');
    try {
      const { request_id } = await api<{ request_id: string }>('/request-access', { method: 'POST' });
      const state = await followRequest(request_id);
      setNote(state === 'allowed' ? 'Approved.' : state === 'denied' ? 'Denied there.' : 'No answer in time. Try again.');
      show(await api<ClientView>(''));
    } catch (e) {
      setNote(`Could not ask: ${(e as Error).message}`);
    } finally {
      setAsking(false);
    }
  };

  const canAsk = view && !view.local && view.state === 'not_approved';
  return (
    <div className="flex flex-col gap-1.5">
      <input
        className={inputCls}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={save}
        onKeyDown={(e) => { if (e.key === 'Enter') save(); }}
        placeholder="http://127.0.0.1:9528"
      />
      {view && <span className="text-xs text-slate-500 dark:text-slate-400">{STATE_TEXT[view.state]}</span>}
      {canAsk && (
        <button
          className="self-start px-3 py-1.5 rounded-lg text-xs font-bold bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50"
          disabled={asking}
          onClick={requestAccess}
        >
          Request access
        </button>
      )}
      {note && <span className="text-xs text-slate-500 dark:text-slate-400">{note}</span>}
    </div>
  );
};

async function followRequest(id: string): Promise<string> {
  for (let i = 0; i < 60; i++) {
    await sleep(2000);
    const { state } = await api<{ state: string }>(`/access/${encodeURIComponent(id)}`);
    if (state !== 'pending') return state;
  }
  return 'expired';
}
