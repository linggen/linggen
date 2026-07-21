import React from 'react';
import { Check, Pencil, RefreshCw, Smartphone, Trash2, X } from 'lucide-react';
import type { AppConfig } from '../types';

const sectionCls ='bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm p-5';
const codeCls = 'font-mono text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-2 py-1';

const LAN_HOST = '0.0.0.0';
const LOOPBACK = '127.0.0.1';

interface PairDevice {
  id: string;
  name: string;
  created_at: number;
  settings?: { models?: string[] };
}

interface PairInfo {
  lan_live: boolean;
  config_host: string;
  port: number;
  lan_ip: string | null;
  mdns_host: string;
  mac_name: string;
  account_name: string | null;
  devices: PairDevice[];
}

interface PairQr {
  svg: string;
  url: string;
  host: string;
}

export const PhoneTab: React.FC<{
  config: AppConfig;
  onChange: (config: AppConfig) => void;
}> = ({ config, onChange }) => {
  const [info, setInfo] = React.useState<PairInfo | null>(null);
  const [qr, setQr] = React.useState<PairQr | null>(null);
  const [confirmingId, setConfirmingId] = React.useState<string | null>(null);
  const [editingId, setEditingId] = React.useState<string | null>(null);
  const [editName, setEditName] = React.useState('');
  const [modelIds, setModelIds] = React.useState<string[]>([]);

  const lanEnabled = config.server.host === LAN_HOST;

  const fetchInfo = React.useCallback(async () => {
    try {
      const resp = await fetch('/api/pair/info');
      if (resp.ok) setInfo(await resp.json());
    } catch {
      /* daemon unreachable — the page-level error covers it */
    }
  }, []);

  const fetchQr = React.useCallback(async () => {
    try {
      const resp = await fetch('/api/pair/qr');
      if (resp.ok) setQr(await resp.json());
    } catch {
      /* ignore */
    }
  }, []);

  // Poll info while the tab is open: a phone pairing via the QR shows up in
  // the device list within a tick, no manual refresh.
  React.useEffect(() => {
    fetchInfo();
    const t = setInterval(fetchInfo, 5000);
    return () => clearInterval(t);
  }, [fetchInfo]);

  // The QR is single-use — mint one when the tab opens (LAN live only),
  // fresh again on demand via the New code button.
  React.useEffect(() => {
    if (info?.lan_live && !qr) fetchQr();
  }, [info?.lan_live, qr, fetchQr]);

  // Available models to offer as each phone's allow-list.
  React.useEffect(() => {
    fetch('/api/models')
      .then((r) => r.json())
      .then((ms) => Array.isArray(ms) && setModelIds(ms.map((m) => m?.id).filter(Boolean)))
      .catch(() => {});
  }, []);

  // Toggle a model in a device's allow-list, then persist via PATCH.
  const toggleModel = async (device: PairDevice, modelId: string) => {
    const current = device.settings?.models ?? [];
    const next = current.includes(modelId)
      ? current.filter((m) => m !== modelId)
      : [...current, modelId];
    // Optimistic UI, then persist.
    setInfo((prev) =>
      prev
        ? { ...prev, devices: prev.devices.map((d) => (d.id === device.id ? { ...d, settings: { ...d.settings, models: next } } : d)) }
        : prev,
    );
    try {
      await fetch(`/api/pair/devices/${device.id}`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ settings: { ...device.settings, models: next } }),
      });
    } finally {
      fetchInfo();
    }
  };

  const setLan = (on: boolean) => {
    onChange({ ...config, server: { ...config.server, host: on ? LAN_HOST : LOOPBACK } });
  };

  const revoke = async (id: string) => {
    if (confirmingId !== id) {
      setConfirmingId(id);
      return;
    }
    setConfirmingId(null);
    try {
      await fetch(`/api/pair/devices/${id}`, { method: 'DELETE' });
    } finally {
      fetchInfo();
    }
  };

  const startEdit = (id: string, name: string) => {
    setEditingId(id);
    setEditName(name);
  };

  const saveEdit = async (id: string) => {
    const name = editName.trim();
    setEditingId(null);
    if (!name) return;
    try {
      await fetch(`/api/pair/devices/${id}`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name }),
      });
    } finally {
      fetchInfo();
    }
  };

  // Saved intent vs live bind — the gap only a restart closes.
  const restartNeeded = info !== null && info.lan_live !== (info.config_host === LAN_HOST);

  return (
    <div className="space-y-6">
      {/* Phone access */}
      <section className={sectionCls}>
        <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-4">Phone Access</h2>
        <div className="flex items-center justify-between gap-4">
          <div>
            <p className="text-sm font-semibold text-slate-800 dark:text-slate-200">Allow phones on your Wi-Fi to connect</p>
            <p className="text-xs text-slate-500 dark:text-slate-400 mt-1">
              Every phone must pair first — a code or QR confirmed at this Mac. Off means the daemon only listens on this Mac.
            </p>
          </div>
          <button
            role="switch"
            aria-checked={lanEnabled}
            onClick={() => setLan(!lanEnabled)}
            className={`relative w-11 h-6 rounded-full transition-colors shrink-0 ${
              lanEnabled ? 'bg-blue-600' : 'bg-slate-300 dark:bg-white/15'
            }`}
          >
            <span
              className={`absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform ${
                lanEnabled ? 'translate-x-5' : ''
              }`}
            />
          </button>
        </div>
        {restartNeeded && (
          <p className="mt-3 text-xs font-semibold text-amber-600 dark:text-amber-400">
            Saved — restart Linggen to {info!.config_host === LAN_HOST ? 'open' : 'close'} phone access.
          </p>
        )}
        {info && info.lan_live && (
          <div className="mt-4 flex flex-wrap items-center gap-2 text-xs text-slate-500 dark:text-slate-400">
            <span>This Mac on the network:</span>
            <code className={codeCls}>{info.mdns_host}:{info.port}</code>
            {info.lan_ip && <code className={codeCls}>{info.lan_ip}:{info.port}</code>}
          </div>
        )}
      </section>

      {/* Pair a phone */}
      <section className={sectionCls}>
        <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-4">Pair a Phone</h2>
        {info?.lan_live ? (
          <div className="flex flex-col sm:flex-row gap-5 items-start">
            {qr ? (
              <div className="bg-white p-3 rounded-xl border border-slate-200 dark:border-transparent shrink-0 [&_svg]:block [&_svg]:w-[200px] [&_svg]:h-[200px]"
                dangerouslySetInnerHTML={{ __html: qr.svg }}
              />
            ) : (
              <div className="w-[224px] h-[224px] rounded-xl bg-slate-100 dark:bg-white/5 animate-pulse shrink-0" />
            )}
            <div className="text-xs text-slate-500 dark:text-slate-400 space-y-2 pt-1">
              <p className="text-sm font-semibold text-slate-800 dark:text-slate-200">
                Scan with the phone's Camera app
              </p>
              <p>It opens Linggen on the phone and pairs with <b>{info.mac_name}</b>{info.account_name ? ` · ${info.account_name}` : ''} — nothing to type.</p>
              <p>Scan it from any number of phones — it stays valid until you tap New code or restart Linggen.</p>
              <p>No camera handy? In the app, pick this Mac under <b>Nearby Macs</b> and type the code that appears on this screen.</p>
              <button
                onClick={fetchQr}
                className="flex items-center gap-1.5 mt-1 px-2.5 py-1.5 rounded-lg text-xs font-semibold bg-slate-100 dark:bg-white/5 hover:bg-slate-200 dark:hover:bg-white/10 text-slate-600 dark:text-slate-300 transition-colors"
              >
                <RefreshCw size={12} /> New code
              </button>
            </div>
          </div>
        ) : (
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Turn on phone access above{restartNeeded ? ' — then restart Linggen —' : ', save,'} and the pairing QR appears here.
          </p>
        )}
      </section>

      {/* Paired devices */}
      <section className={sectionCls}>
        <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-4">Paired Devices</h2>
        {info === null ? (
          <p className="text-xs text-slate-400">Loading…</p>
        ) : info.devices.length === 0 ? (
          <p className="text-xs text-slate-500 dark:text-slate-400">No devices paired yet.</p>
        ) : (
          <ul className="divide-y divide-slate-100 dark:divide-white/5">
            {info.devices.map((d) => (
              <li key={d.id} className="py-2.5">
                <div className="group flex items-center gap-3">
                <Smartphone size={16} className="text-slate-400 shrink-0" />
                <div className="flex-1 min-w-0">
                  {editingId === d.id ? (
                    <div className="flex items-center gap-1.5">
                      <input
                        autoFocus
                        value={editName}
                        onChange={(e) => setEditName(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') saveEdit(d.id);
                          if (e.key === 'Escape') setEditingId(null);
                        }}
                        maxLength={64}
                        className="flex-1 min-w-0 bg-slate-100 dark:bg-white/5 border border-slate-300 dark:border-white/15 rounded px-2 py-1 text-sm outline-none focus:ring-1 focus:ring-blue-500/50"
                      />
                      <button onClick={() => saveEdit(d.id)} className="p-1 text-green-600 hover:bg-green-50 dark:hover:bg-green-500/10 rounded" title="Save">
                        <Check size={14} />
                      </button>
                      <button onClick={() => setEditingId(null)} className="p-1 text-slate-400 hover:bg-slate-100 dark:hover:bg-white/10 rounded" title="Cancel">
                        <X size={14} />
                      </button>
                    </div>
                  ) : (
                    <button
                      onClick={() => startEdit(d.id, d.name)}
                      className="flex items-center gap-1.5 text-left group/name"
                      title="Rename"
                    >
                      <span className="text-sm font-medium text-slate-800 dark:text-slate-200 truncate">{d.name}</span>
                      <Pencil size={11} className="text-slate-400 opacity-0 group-hover:opacity-100 transition-opacity shrink-0" />
                    </button>
                  )}
                  <p className="text-[11px] text-slate-400">
                    Paired {new Date(d.created_at * 1000).toLocaleDateString()}
                  </p>
                </div>
                {editingId !== d.id && (
                  <button
                    onClick={() => revoke(d.id)}
                    onBlur={() => setConfirmingId((c) => (c === d.id ? null : c))}
                    className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-semibold transition-colors ${
                      confirmingId === d.id
                        ? 'bg-red-600 text-white hover:bg-red-700'
                        : 'text-slate-500 hover:text-red-600 hover:bg-red-50 dark:hover:bg-red-500/10'
                    }`}
                  >
                    <Trash2 size={12} />
                    {confirmingId === d.id ? 'Confirm revoke' : 'Revoke'}
                  </button>
                )}
                </div>
                {editingId !== d.id && (
                  <div className="mt-2.5 ml-7 pl-3 border-l-2 border-slate-100 dark:border-white/5">
                    <p className="text-[11px] text-slate-500 dark:text-slate-400 mb-1.5">
                      Which models this phone may use. The phone picks one from this set.
                    </p>
                    {modelIds.length === 0 ? (
                      <p className="text-[11px] text-slate-400">No models configured.</p>
                    ) : (
                      <div className="flex flex-wrap gap-1.5">
                        {modelIds.map((m) => {
                          const on = (d.settings?.models ?? []).includes(m);
                          return (
                            <button
                              key={m}
                              onClick={() => toggleModel(d, m)}
                              className={`px-2 py-1 rounded-md text-[11px] font-medium border transition-colors ${
                                on
                                  ? 'bg-blue-600/10 border-blue-500/40 text-blue-600 dark:text-blue-400'
                                  : 'border-slate-200 dark:border-white/10 text-slate-500 hover:border-slate-300 dark:hover:border-white/20'
                              }`}
                            >
                              {on ? '✓ ' : ''}{m}
                            </button>
                          );
                        })}
                      </div>
                    )}
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
        <p className="mt-3 text-[11px] text-slate-400">
          Revoking cuts the device off immediately — it re-pairs with eyes on this Mac. Models sync to the phone one-way when it taps Sync.
        </p>
      </section>
    </div>
  );
};
