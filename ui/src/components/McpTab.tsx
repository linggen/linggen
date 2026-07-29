import React, { useCallback, useEffect, useState } from 'react';
import { AlertTriangle, Check, Circle, Plug, RefreshCw, ShieldAlert } from 'lucide-react';

/**
 * MCP servers this engine connects to as a *client*.
 *
 * The rule this screen follows: show the server's state, never a prettier
 * version of it. Three states are genuinely different and each is rendered as
 * itself — connected, tried-and-failed (with the reason), and parked. A failed
 * server that simply vanished would read as "not configured", which is a
 * different and misleading thing.
 */

type Scope = 'user' | 'project';

interface McpServer {
  name: string;
  scope: Scope;
  /** `stdio: npx …` or the URL — what you recognise it by. */
  target: string;
  enabled: boolean;
  connected: boolean;
  error: string | null;
  tools: string[];
  tier: string;
}

const scopeBadge: Record<Scope, string> = {
  user: 'bg-indigo-500/10 text-indigo-600 dark:text-indigo-400 border-indigo-200/50 dark:border-indigo-500/20',
  project:
    'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border-emerald-200/50 dark:border-emerald-500/20',
};

export const McpTab: React.FC = () => {
  const [servers, setServers] = useState<McpServer[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const res = await fetch('/api/mcp');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      setServers(data.servers || []);
      setLoadError(null);
    } catch (e) {
      setLoadError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  if (loadError) {
    return (
      <div className="text-sm text-amber-600 dark:text-amber-400">
        Couldn't read the MCP servers: {loadError}
      </div>
    );
  }
  if (servers === null) return <div className="text-sm opacity-60">Loading…</div>;

  return (
    <div className="space-y-4">
      <div className="flex items-start justify-between gap-4">
        <p className="text-sm opacity-70 max-w-2xl">
          Servers Linggen connects to for extra tools. Add them in{' '}
          <code className="text-xs">linggen.runtime.toml</code> under{' '}
          <code className="text-xs">[mcp_servers]</code>, in the same shape Claude
          Code and Cursor use. A repo's own <code className="text-xs">.mcp.json</code>{' '}
          is picked up automatically and shown here read-only.
        </p>
        <button
          onClick={load}
          className="shrink-0 inline-flex items-center gap-1.5 text-xs px-2.5 py-1.5 rounded-md border border-slate-200 dark:border-slate-700 hover:bg-slate-50 dark:hover:bg-slate-800"
        >
          <RefreshCw size={13} /> Refresh
        </button>
      </div>

      {servers.length === 0 && (
        <div className="text-sm opacity-60 border border-dashed border-slate-200 dark:border-slate-700 rounded-lg p-6 text-center">
          No MCP servers configured.
        </div>
      )}

      {servers.map((s) => (
        <ServerCard key={`${s.scope}:${s.name}`} server={s} />
      ))}

      {servers.some((s) => s.connected) && (
        <p className="flex items-start gap-2 text-xs opacity-60 pt-1">
          <ShieldAlert size={13} className="mt-0.5 shrink-0" />
          <span>
            Tools from these servers run at the <strong>admin</strong> tier and are
            permission-gated like any other admin action — they can write files and
            call out to the network. Only add servers you trust.
          </span>
        </p>
      )}
    </div>
  );
};

const ServerCard: React.FC<{ server: McpServer }> = ({ server: s }) => {
  // Three distinct states, rendered as themselves. A parked server carries no
  // error because it was never tried — inventing one would be a lie about state.
  const state = !s.enabled ? 'parked' : s.connected ? 'connected' : 'failed';

  const accent =
    state === 'connected'
      ? 'border-l-emerald-400 dark:border-l-emerald-500'
      : state === 'failed'
        ? 'border-l-amber-400 dark:border-l-amber-500'
        : 'border-l-slate-300 dark:border-l-slate-600';

  return (
    <div
      className={`border border-l-2 ${accent} border-slate-200 dark:border-slate-700 rounded-lg p-3.5 space-y-2`}
    >
      <div className="flex items-center gap-2 flex-wrap">
        <Plug size={14} className="opacity-60 shrink-0" />
        <span className="font-medium text-sm">{s.name}</span>

        <span
          className={`text-[10px] px-1.5 py-0.5 rounded border ${scopeBadge[s.scope]}`}
        >
          {s.scope === 'project' ? 'from this repo' : 'yours'}
        </span>

        {state === 'connected' && (
          <span className="inline-flex items-center gap-1 text-[11px] text-emerald-600 dark:text-emerald-400">
            <Check size={12} /> connected · {s.tools.length} tool
            {s.tools.length === 1 ? '' : 's'}
          </span>
        )}
        {state === 'failed' && (
          <span className="inline-flex items-center gap-1 text-[11px] text-amber-600 dark:text-amber-400">
            <AlertTriangle size={12} /> not reachable
          </span>
        )}
        {state === 'parked' && (
          <span className="inline-flex items-center gap-1 text-[11px] opacity-50">
            <Circle size={11} /> off
          </span>
        )}
      </div>

      <div className="text-xs opacity-60 font-mono break-all">{s.target}</div>

      {/* The reason, verbatim. A server that failed is only actionable if you
          can see why it failed. */}
      {s.error && (
        <div className="text-xs text-amber-600 dark:text-amber-400 break-all">
          {s.error}
        </div>
      )}

      {s.scope === 'project' && (
        <div className="text-xs opacity-50">
          Defined by this project's <code>.mcp.json</code> — edit it there. A server
          you also define yourself takes precedence.
        </div>
      )}

      {s.tools.length > 0 && (
        <div className="flex flex-wrap gap-1 pt-0.5">
          {s.tools.map((t) => (
            <code
              key={t}
              className="text-[10px] px-1.5 py-0.5 rounded bg-slate-500/8 border border-slate-200/50 dark:border-slate-600/40"
            >
              {t}
            </code>
          ))}
        </div>
      )}
    </div>
  );
};
