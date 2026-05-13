import React, { useMemo, useEffect } from 'react';
import { SessionList } from '../../components/SessionList';
import { useSessionStore } from '../../stores/sessionStore';
import { useOpenSettings } from '../../hooks/useOpenSettings';

/** Bare /sessions route — for skill apps to iframe the session list alone.
 *
 *  URL params:
 *    skill   — when set, filter to this skill's sessions only, lock filter
 *              tabs + missions section away, and post `session_select` /
 *              `session_create` events up to the parent window so the host
 *              page can route. Used by pulse + sys-doctor.
 *    active  — overrides the highlighted row (host's currently-viewed
 *              session, which may differ from the chat session because the
 *              host navigates via URL on click).
 *
 *  Selection writes to the global session store when no `skill` param is
 *  set; with `skill` set, selection is purely a postMessage so the host
 *  owns navigation. */
export const BareSessions: React.FC = () => {
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const allSessions = useSessionStore((s) => s.allSessions);
  const sessionStore = useSessionStore();
  const openSettings = useOpenSettings();

  const params = new URLSearchParams(window.location.search);
  const skillParam = params.get('skill') || '';
  const activeParam = params.get('active') || '';

  // When mounted inside a skill app iframe, filter to that skill only.
  const filtered = useMemo(() => {
    if (!skillParam) return undefined;
    return allSessions.filter(s => s.creator === 'skill' && (s.skill || '') === skillParam);
  }, [allSessions, skillParam]);

  // Skill-bound active highlight overrides the store's activeSessionId so
  // the host's "currently-viewed" session shows selected even when the
  // active chat session is a different one.
  const effectiveActive = skillParam ? (activeParam || null) : activeSessionId;

  // Post bridge events to host page when running in a skill iframe.
  const postToHost = (event: string, payload: unknown) => {
    if (window.parent !== window) {
      window.parent.postMessage({ type: 'linggen-skill-event', event, payload }, '*');
    }
  };

  const handleSelect = (session: { id: string }) => {
    if (skillParam) {
      postToHost('session_select', { sessionId: session.id });
      return;
    }
    sessionStore.setActiveSessionId(session.id);
    window.localStorage.setItem('linggen:active-session', session.id);
  };

  const handleCreate = async () => {
    if (skillParam) {
      // Host page owns the new-session flow (it needs to navigate + replay
      // runtime grants). Engine creates the session; iframe just signals.
      try {
        const r = await fetch('/api/sessions', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ title: `${skillParam} session`, skill: skillParam }),
        });
        const data = await r.json();
        if (data?.id) postToHost('session_create', { sessionId: data.id });
      } catch (e) {
        console.warn('[bare-sessions] create failed', e);
      }
      return;
    }
    sessionStore.createSession();
  };

  // Listen for host pushing a new active sid (e.g. after the host
  // navigates without a full iframe reload).
  useEffect(() => {
    if (!skillParam) return;
    const onMessage = (e: MessageEvent) => {
      if (e.data?.type !== 'linggen-skill') return;
      if (e.data.action === 'set_active' && e.data.payload?.sessionId) {
        const sid = e.data.payload.sessionId;
        // Reflect in URL so the active highlight survives reload.
        const u = new URL(window.location.href);
        u.searchParams.set('active', sid);
        window.history.replaceState({}, '', u.toString());
        // Force re-render by nudging the store (no real state change needed
        // — effectiveActive recomputes on every render anyway, but the
        // URL-only update doesn't trigger React, so bump activeSessionId).
        useSessionStore.setState({ activeSessionId: sid });
      }
    };
    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  }, [skillParam]);

  return (
    <div className="h-screen w-screen bg-white dark:bg-[#0f0f0f] flex flex-col">
      <SessionList
        activeSessionId={effectiveActive}
        onSelectSession={handleSelect}
        onCreateSession={handleCreate}
        onDeleteSession={(id) => sessionStore.removeSession(id)}
        onOpenSettings={skillParam ? undefined : (tab) => openSettings(tab as any)}
        filterSessions={filtered}
        hideMissions={!!skillParam}
        hideFilters={!!skillParam}
      />
    </div>
  );
};
