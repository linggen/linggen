import { useNavigate } from 'react-router-dom';
import { useSessionStore } from '../../stores/sessionStore';
import { showChatHome } from '../../lib/appHost';

/** Open a mission run using the main session list's selection state. */
export function OpenRun({ sessionId }: { sessionId: string }) {
  const navigate = useNavigate();
  const open = () => {
    const store = useSessionStore.getState();
    const session = store.allSessions.find((item) => item.id === sessionId);
    store.setActiveSessionId(sessionId);
    store.setIsMissionSession(true);
    store.setIsSkillSession(false);
    store.setActiveSkillName(null);
    store.setActiveMissionId(session?.mission_id ?? null);
    if (session) store.setSelectedProjectRoot(session.project || session.cwd || session.repo_path || '~');
    showChatHome();
    navigate('/');
  };
  return <button className="text-xs text-blue-600 underline dark:text-blue-400" onClick={open}>Open the run</button>;
}
