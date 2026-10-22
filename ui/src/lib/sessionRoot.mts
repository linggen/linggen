// The folder the open session works in, as the page mirrors it
// (`sessionStore.selectedProjectRoot`). It is always the open session's own:
// set when a session is picked, restored or made, and never carried to the
// next one — a New chat starts where the engine put it (the default
// workspace), not in the folder of the chat open before. Pure, so node tests
// can hold it.

export interface RootedSession {
  id: string;
  project?: string | null;
  cwd?: string | null;
  repo_path?: string | null;
}

/** A session's working folder: its project, else its cwd; '' when unknown. */
export function sessionRootOf(session: RootedSession | null | undefined): string {
  if (!session) return '';
  return session.project || session.cwd || session.repo_path || '';
}

export interface MadeSession { id: string; title: string; cwd?: string | null }

/** The list row a made session gets until page_state brings the full one. */
export interface MadeEntry { id: string; repo_path: string; title: string; created_at: number; cwd: string }

/** The store after a New chat: the made session is open, its folder (the one
 *  the engine chose) replaces whatever folder the previous chat had. */
export function afterNewChat<T>(
  allSessions: T[],
  made: MadeSession,
  nowSecs: number,
): { activeSessionId: string; selectedProjectRoot: string; allSessions: (T | MadeEntry)[] } {
  const cwd = made.cwd || '';
  const entry: MadeEntry = { id: made.id, repo_path: '', title: made.title, created_at: nowSecs, cwd };
  return { activeSessionId: made.id, selectedProjectRoot: cwd, allSessions: [entry, ...allSessions] };
}
