/**
 * Compact per-session model and permission mode selectors + stats.
 * Extracted from ChatPanel.tsx.
 */
import React from 'react';
import { chatAgentOf, withMemberModel } from '../../lib/sessionMembers.mts';
import { formatTokens as fmt } from '../../lib/paneItems.mts';
import { useServerStore } from '../../stores/serverStore';
import { useUiStore } from '../../stores/uiStore';
import { useUserStore } from '../../stores/userStore';
import { useSessionStore } from '../../stores/sessionStore';
import { suppressPermissionSync } from '../../lib/eventDispatcher';
import { apiErrorMessage } from '../../lib/api';
import { sessionApi } from '../../lib/endpoints';
import { translate } from '../../lib/i18n';
import { EFFORT_WORD, effortChip } from '../../lib/effortChip.mts';
import { W } from '../../lib/modelsWords.mts';
import type { SessionInfo } from '../../types';
import { SessionSpend } from './SessionSpend';
import { isOldClaude } from '../../lib/oldClaude.mts';


/** Compact context usage display for the session top bar. */
export const SessionStats: React.FC = () => {
  const sessionId = useSessionStore((s) => s.activeSessionId) || '';
  const ctx = useServerStore((s) => s.agentContext[sessionId]);
  const tokens = ctx?.tokens || 0;
  const limit = ctx?.tokenLimit && ctx.tokenLimit > 0 ? ctx.tokenLimit : 0;
  const pct = limit ? Math.round((tokens / limit) * 100) : 0;

  return (
    <span className="text-[10px] font-mono text-slate-400 ml-auto shrink-0" title={`Context: ${tokens} / ${limit || '?'} tokens (${pct}%)`}>
      {tokens > 0 && <><span className={pct > 80 ? 'text-red-400' : pct > 50 ? 'text-amber-400' : ''}>{fmt(tokens)}</span>
      {limit > 0 && <span className="text-slate-300 dark:text-slate-600">/{fmt(limit)}</span>}
      {limit > 0 && <span className={pct > 80 ? 'text-red-400' : pct > 50 ? 'text-amber-400' : ''}> {pct}%</span>}</>}
      <SessionSpend leading={tokens > 0} />
    </span>
  );
};

/** Compact per-session model selector shown in the run bar. */
export const SessionModelSelector: React.FC = () => {
  const models = useServerStore((s) => s.models);
  const session = useSessionStore((s) => s.allSessions.find((session) => session.id === s.activeSessionId));
  const defaultModels = useServerStore((s) => s.defaultModels);
  const sessionModel = useUiStore((s) => s.sessionModel);
  const setSessionModel = useUiStore((s) => s.setSessionModel);
  const sessionId = useSessionStore((s) => s.activeSessionId);
  const selectedProjectRoot = useSessionStore((s) => s.selectedProjectRoot);
  const userType = useUserStore((s) => s.userType);

  // Consumers (proxy room) only see models the owner has shared. There's no
  // "owner default" available to them — the server picks the first shared
  // model when no override is sent. Use that as the Default label.
  const isConsumer = userType === 'consumer';
  const defaultLabel = isConsumer
    ? (models[0]?.id ?? 'shared')
    : (defaultModels.length > 0 ? defaultModels[0] : 'default');

  if (isOldClaude(session)) return null;

  // The picker is the model of the member this chat speaks with; each
  // member keeps its own.
  const handleChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const value = e.target.value || null;
    setSessionModel(value);
    if (sessionId) {
      const ui = useUiStore.getState();
      const agent = chatAgentOf(useServerStore.getState().selectedAgent, ui.sessionMembers);
      const members = withMemberModel(ui.sessionMembers, agent, value);
      ui.setSessionMembers(members);
      const ps = useSessionStore.getState();
      const withMembers = (s: SessionInfo) => (s.id === sessionId ? { ...s, agents: members } : s);
      useSessionStore.setState({
        allSessions: ps.allSessions.map(withMembers),
        sessions: ps.sessions.map(withMembers),
      });
      sessionApi.setModel(selectedProjectRoot || '', sessionId, value ?? '', agent).catch(() => {});
    }
  };

  return (
    <select
      data-testid="session-model"
      value={sessionModel ?? ''}
      onChange={handleChange}
      onClick={(e) => e.stopPropagation()}
      className="ml-auto text-[11px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 outline-none max-w-[14rem] truncate"
      title="Session model override"
    >
      <option value="">Default ({defaultLabel})</option>
      {/* List every model explicitly — including the global default. With the
          per-skill `model:` tier, "Default" resolves to the skill's pinned
          model for app sessions, so the global default must be selectable on
          its own; otherwise a user whose skill model hit its limit can never
          switch off it (the only escape is an explicit, non-"Default" pin). */}
      {models.map((m) => (
        <option key={m.id} value={m.id}>
          {m.id}{m.provided_by ? ` (${m.provided_by})` : ''}{defaultModels.includes(m.id) ? ' — default' : ''}{m.auth_ok === false ? ' — sign in required' : ''}
        </option>
      ))}
    </select>
  );
};

/** The conversation's thinking effort, beside its model: only the values the
 *  model offers (the engine's list), nothing when it offers none. 自动 = the chat
 *  default. A kept value the model lacks reads as 自动 and stays kept. */
export const SessionEffortChip: React.FC = () => {
  const models = useServerStore((s) => s.models);
  const defaultModels = useServerStore((s) => s.defaultModels);
  const sessionModel = useUiStore((s) => s.sessionModel);
  const sessionId = useSessionStore((s) => s.activeSessionId);
  const session = useSessionStore((s) => s.allSessions.find((item) => item.id === s.activeSessionId));
  if (!sessionId || !session || isOldClaude(session)) return null;
  const chip = effortChip(models, sessionModel ?? defaultModels[0], session.effort);
  if (!chip) return null;
  const t = (text: string) => translate(text) ?? text;
  const word = (value: string) => `${t(W.thinking)}: ${t(value ? EFFORT_WORD[value] : W.automatic)}`;
  const change = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const value = e.target.value;
    const previous = session.effort ?? null;
    const set = (effort: string | null) => (item: SessionInfo) => (item.id === sessionId ? { ...item, effort } : item);
    useSessionStore.setState((s) => ({ allSessions: s.allSessions.map(set(value || null)), sessions: s.sessions.map(set(value || null)) }));
    const ui = useUiStore.getState();
    const agent = chatAgentOf(useServerStore.getState().selectedAgent, ui.sessionMembers);
    sessionApi.setEffort(sessionId, value, agent).catch((error) => {
      useSessionStore.setState((s) => ({ allSessions: s.allSessions.map(set(previous)), sessions: s.sessions.map(set(previous)) }));
      useUiStore.getState().addToast({ variant: 'error', message: apiErrorMessage(error) });
    });
  };
  return (
    <select
      data-testid="session-effort"
      value={chip.selected}
      onChange={change}
      onClick={(e) => e.stopPropagation()}
      className="text-[11px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 outline-none"
    >
      {['', ...chip.values].map((value) => <option key={value || 'auto'} value={value}>{word(value)}</option>)}
    </select>
  );
};

/** One-line switch: may a failed model call be answered by another model in this session? */
export const SessionFallbackToggle: React.FC = () => {
  const sessionId = useSessionStore((s) => s.activeSessionId);
  const session = useSessionStore((s) => s.allSessions.find((item) => item.id === s.activeSessionId));
  if (!sessionId || !session || isOldClaude(session)) return null;
  const on = session.model_fallback !== false;
  const change = () => {
    const allowed = !on;
    const update = (item: SessionInfo) => (item.id === sessionId ? { ...item, model_fallback: allowed } : item);
    useSessionStore.setState((s) => ({ allSessions: s.allSessions.map(update), sessions: s.sessions.map(update) }));
    sessionApi.setModelFallback(sessionId, allowed).catch((error) => {
      const revert = (item: SessionInfo) => (item.id === sessionId ? { ...item, model_fallback: !allowed } : item);
      useSessionStore.setState((s) => ({ allSessions: s.allSessions.map(revert), sessions: s.sessions.map(revert) }));
      useUiStore.getState().addToast({ variant: 'error', message: apiErrorMessage(error) });
    });
  };
  return (
    <button
      type="button"
      className={`text-[11px] border rounded px-1.5 py-0.5 font-semibold ${on
        ? 'border-current bg-white dark:bg-black/30 text-blue-600 dark:text-blue-400'
        : 'border-slate-200 dark:border-white/10 text-slate-300 dark:text-slate-600'}`}
      onClick={(e) => { e.stopPropagation(); change(); }}
      aria-pressed={on}
      title={on ? 'On: when the model fails, another model answers.' : 'Off: when the model fails, the run ends on that model.'}
    >
      Auto switch
    </button>
  );
};

/** Coding (guidance, and programs run in its folder without asking) is a
 *  switch on a session of a kind that takes a work tag
 *  (`takes_work`, decided by the engine's session spec — never re-derived here). */
export const SessionCodingToggle: React.FC = () => {
  const sessionId = useSessionStore((s) => s.activeSessionId);
  const session = useSessionStore((s) => s.allSessions.find((item) => item.id === s.activeSessionId));
  if (!sessionId || !session?.takes_work) return null;
  const on = session.work === 'code';
  const change = () => {
    const work: SessionInfo['work'] = on ? null : 'code';
    const update = (item: SessionInfo) => item.id === sessionId ? { ...item, work } : item;
    useSessionStore.setState((s) => ({ allSessions: s.allSessions.map(update), sessions: s.sessions.map(update) }));
    sessionApi.setWork(sessionId, work ?? 'none').catch((error) => {
      const revert = (item: SessionInfo) => item.id === sessionId ? { ...item, work: session.work } : item;
      useSessionStore.setState((s) => ({ allSessions: s.allSessions.map(revert), sessions: s.sessions.map(revert) }));
      useUiStore.getState().addToast({ variant: 'error', message: apiErrorMessage(error) });
    });
  };
  return <button type="button"
    className={`text-[11px] border rounded px-1.5 py-0.5 font-semibold ${on
      ? 'border-current bg-white dark:bg-black/30 text-blue-600 dark:text-blue-400'
      : 'border-slate-200 dark:border-white/10 text-slate-300 dark:text-slate-600'}`}
    onClick={(e) => { e.stopPropagation(); change(); }} aria-pressed={on}
    title="Adds how to work on code, and lets this chat run programs in its folder without asking.">Coding</button>;
};

/** Compact per-session permission mode selector shown in the run bar. */
export const SessionModeSelector: React.FC = () => {
  const sessionMode = useUiStore((s) => s.sessionMode);
  const setSessionMode = useUiStore((s) => s.setSessionMode);
  const userPermission = useUserStore((s) => s.userPermission);
  const userType = useUserStore((s) => s.userType);
  const sessionId = useSessionStore((s) => s.activeSessionId);
  const allMode = useUiStore((s) => s.sessionAllMode);

  const session = useSessionStore((s) => s.allSessions.find((session) => session.id === sessionId));
  if (isOldClaude(session)) return null;

  // Four modes. `chat` means no path-mode grant covers cwd → no tools.
  const modes = [
    { value: 'chat', label: 'chat', color: 'text-slate-500 dark:text-slate-400' },
    { value: 'read', label: 'read', color: 'text-emerald-600 dark:text-emerald-400' },
    { value: 'edit', label: 'edit', color: 'text-blue-600 dark:text-blue-400' },
    { value: 'admin', label: 'admin', color: 'text-amber-600 dark:text-amber-400' },
  ];

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
      sessionApi.setPermission(sessionId, cwd, value).catch(() => {});
    }
  };

  const effective = sessionMode || 'chat';
  const current = modes.find((m) => m.value === effective);

  return (
    <>
      <select
        value={effective}
        onChange={handleChange}
        onClick={(e) => e.stopPropagation()}
        className={`text-[11px] border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 outline-none font-semibold bg-white dark:bg-black/30 ${current?.color || ''}`}
        title="Permission mode for this folder"
      >
        {modes.map((m) => (
          <option key={m.value} value={m.value}>{m.label}</option>
        ))}
      </select>
      <AllFoldersBadge mode={allMode} color={modes.find((m) => m.value === allMode)?.color} />
    </>
  );
};

/** Lit when the session holds a grant on every folder (`*`); its colour is that grant's mode. */
const AllFoldersBadge: React.FC<{ mode: string | null; color?: string }> = ({ mode, color }) => (
  <span
    className={`text-[11px] border rounded px-1.5 py-0.5 font-semibold ${mode
      ? `border-current bg-white dark:bg-black/30 ${color || ''}`
      : 'border-slate-200 dark:border-white/10 text-slate-300 dark:text-slate-600'}`}
    title={mode ? `All folders: ${mode}` : 'No grant on all folders'}
  >
    all
  </span>
);

