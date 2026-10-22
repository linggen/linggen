/**
 * A fresh session in place — the way an app page (its toolbar hidden) gets
 * a new chat, and the embed toolbar's "+". A skill session stays bound to its
 * skill and to the values the page mounted with (`sessionStore.boundParams`);
 * the parent page is told the way a first send tells it, so its bridge
 * follows the new id. Resolves to the new id, or null when it failed.
 */
import { sessions as sessionsApi } from './api';
import { postToParent } from './parentFrame';
import { newChatRequest } from './sessionBinding.mts';
import { useSessionStore } from '../stores/sessionStore';
import { useChatStore } from '../stores/chatStore';

function chatTitle(skill?: string): string {
  if (skill) return `${skill} session`;
  const now = new Date();
  return `Chat ${now.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}, ${now.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })}`;
}

export async function startNewChat(): Promise<string | null> {
  const ss = useSessionStore.getState();
  const skill = ss.isSkillSession && ss.activeSkillName ? ss.activeSkillName : undefined;
  const req = newChatRequest(chatTitle(skill), skill, ss.boundParams);
  // A message sent before the session exists waits for it (newChat).
  const making = sessionsApi.create(req).then((data) => {
    const created = {
      id: data.id, repo_path: '', title: req.title, created_at: Math.floor(Date.now() / 1000),
      ...(req.skill ? { skill: req.skill } : {}),
      ...(req.params ? { params: req.params } : {}),
    };
    // A plain chat's folder is the one the engine chose; a skill embed keeps
    // '' so its calls use the session's own (EmbedApp).
    const root = req.skill ? '' : data.cwd || '';
    useSessionStore.setState((s) => ({
      activeSessionId: data.id,
      selectedProjectRoot: root,
      allSessions: [{ ...created, ...(data.cwd ? { cwd: data.cwd } : {}) }, ...s.allSessions],
    }));
    const cs = useChatStore.getState();
    cs.setActiveSession(data.id);
    cs.fetchSessionState();
    postToParent({ type: 'linggen-skill-event', event: 'session_created', payload: { sessionId: data.id } });
    return data.id as string;
  }, (err) => {
    console.error('[new-chat] failed:', err);
    return null;
  });
  useSessionStore.setState({ newChat: making });
  await making;
  if (useSessionStore.getState().newChat === making) useSessionStore.setState({ newChat: null });
  return making;
}
