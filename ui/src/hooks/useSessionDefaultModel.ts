import { useEffect, useState } from 'react';
import { sessionApi } from '../lib/endpoints';

/** The engine's answer to "which model does this session's turn run on when
 *  nothing is picked?" (null while unknown). It is asked again when the
 *  session changes, or `changed` does — the routing list, a sign-in, the
 *  session's own settings: whatever the engine's answer may follow. */
export function useSessionDefaultModel(sessionId: string | null | undefined, changed: string): string | null {
  const [answer, setAnswer] = useState<{ sessionId: string; model: string | null } | null>(null);
  useEffect(() => {
    if (!sessionId) return;
    let current = true;
    sessionApi.defaultModel(sessionId)
      .then((r) => { if (current) setAnswer({ sessionId, model: r.model }); })
      .catch(() => { if (current) setAnswer(null); });
    return () => { current = false; };
  }, [sessionId, changed]);
  return answer && answer.sessionId === sessionId ? answer.model : null;
}
