/**
 * Compact per-session model and permission mode selectors + stats.
 * Extracted from ChatPanel.tsx.
 */
import React from 'react';
import { useServerStore } from '../../stores/serverStore';
import { useUiStore } from '../../stores/uiStore';
import { useUserStore } from '../../stores/userStore';
import { useSessionStore } from '../../stores/sessionStore';
import { suppressPermissionSync } from '../../lib/eventDispatcher';

const fmt = (n: number) => n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);

/** Compact context usage display for the session top bar. */
export const SessionStats: React.FC = () => {
  const sessionId = useSessionStore((s) => s.activeSessionId) || '';
  const ctx = useServerStore((s) => s.agentContext[sessionId]);

  const tokens = ctx?.tokens || 0;
  const limit = ctx?.tokenLimit && ctx.tokenLimit > 0 ? ctx.tokenLimit : 0;
  const pct = limit ? Math.round((tokens / limit) * 100) : 0;

  if (!tokens) return null;

  return (
    <span className="text-[10px] font-mono text-slate-400 ml-auto shrink-0" title={`Context: ${tokens} / ${limit || '?'} tokens (${pct}%)`}>
      <span className={pct > 80 ? 'text-red-400' : pct > 50 ? 'text-amber-400' : ''}>{fmt(tokens)}</span>
      {limit > 0 && <span className="text-slate-300 dark:text-slate-600">/{fmt(limit)}</span>}
    </span>
  );
};

/** Compact per-session model selector shown in the run bar. */
export const SessionModelSelector: React.FC = () => {
  const models = useServerStore((s) => s.models);
  const defaultModels = useServerStore((s) => s.defaultModels);
  const sessionModel = useUiStore((s) => s.sessionModel);
  const setSessionModel = useUiStore((s) => s.setSessionModel);
  const sessionId = useSessionStore((s) => s.activeSessionId);
  const selectedProjectRoot = useSessionStore((s) => s.selectedProjectRoot);

  const defaultLabel = defaultModels.length > 0 ? defaultModels[0] : 'default';

  const handleChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const value = e.target.value || null;
    setSessionModel(value);
    if (sessionId) {
      const ps = useSessionStore.getState();
      const updated = ps.allSessions.map((s) =>
        s.id === sessionId ? { ...s, model_id: value } : s
      );
      const updatedSessions = ps.sessions.map((s) =>
        s.id === sessionId ? { ...s, model_id: value } : s
      );
      useSessionStore.setState({ allSessions: updated, sessions: updatedSessions });
      fetch('/api/sessions', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: selectedProjectRoot || '',
          session_id: sessionId,
          model_id: value ?? '',
        }),
      }).catch(() => {});
    }
  };

  return (
    <select
      value={sessionModel ?? ''}
      onChange={handleChange}
      onClick={(e) => e.stopPropagation()}
      className="ml-auto text-[11px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 outline-none max-w-[14rem] truncate"
      title="Session model override"
    >
      <option value="">Default ({defaultLabel})</option>
      {models.filter((m) => !defaultModels.includes(m.id)).map((m) => (
        <option key={m.id} value={m.id}>
          {m.id}{m.provided_by ? ` (${m.provided_by})` : ''}
        </option>
      ))}
    </select>
  );
};

/** Compact per-session permission mode selector shown in the run bar. */
export const SessionModeSelector: React.FC = () => {
  const sessionMode = useUiStore((s) => s.sessionMode);
  const setSessionMode = useUiStore((s) => s.setSessionMode);
  const sessionZone = useUiStore((s) => s.sessionZone);
  const userPermission = useUserStore((s) => s.userPermission);
  const userType = useUserStore((s) => s.userType);
  const sessionId = useSessionStore((s) => s.activeSessionId);

  const modes = [
    { value: 'read', label: 'read', color: 'text-emerald-600 dark:text-emerald-400' },
    { value: 'edit', label: 'edit', color: 'text-blue-600 dark:text-blue-400' },
    { value: 'admin', label: 'admin', color: 'text-amber-600 dark:text-amber-400' },
  ];

  const isSystemZone = sessionZone === 'system';

  // Consumers: show permission as read-only badge (room settings control permissions)
  if (userType === 'consumer') {
    const current = modes.find((m) => m.value === (sessionMode || userPermission));
    return (
      <span
        className={`text-[11px] border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 font-semibold bg-slate-50 dark:bg-black/30 ${current?.color || 'text-slate-400'}`}
        title={`Permission: ${userPermission}`}
      >
        {sessionMode || userPermission}
      </span>
    );
  }

  const handleChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const value = e.target.value;
    setSessionMode(value);
    suppressPermissionSync();
    if (sessionId) {
      const sessionMeta = useSessionStore.getState().allSessions.find((s) => s.id === sessionId);
      const cwd = sessionMeta?.cwd || sessionMeta?.project || '~/';
      fetch('/api/sessions/permission', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ session_id: sessionId, path: cwd, mode: value }),
      }).catch(() => {});
    }
  };

  const current = modes.find((m) => m.value === (sessionMode || 'read'));

  if (isSystemZone) {
    return (
      <span
        className="text-[11px] border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 font-semibold bg-slate-50 dark:bg-black/30 text-slate-400 dark:text-slate-500"
        title="System path — read only, no mode switch"
      >
        read (system)
      </span>
    );
  }

  return (
    <select
      value={sessionMode || 'read'}
      onChange={handleChange}
      onClick={(e) => e.stopPropagation()}
      className={`text-[11px] border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 outline-none font-semibold bg-white dark:bg-black/30 ${current?.color || ''}`}
      title="Session permission mode"
    >
      {modes.map((m) => (
        <option key={m.value} value={m.value}>{m.label}</option>
      ))}
    </select>
  );
};
