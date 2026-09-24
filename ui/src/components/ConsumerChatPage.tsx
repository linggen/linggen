/**
 * Consumer chat page — shown when a browser consumer connects to a proxy room.
 * Reuses ChatWidget for the full chat experience (tools, plan, askuser, etc.)
 * but strips away owner-only chrome (settings, file browser, project selector).
 *
 * Layout mirrors the owner's main page: header + left sidebar (sessions) + center (chat)
 * + right sidebar (allowed skills). No settings button, no file browser.
 */
import React, { useCallback, useState } from 'react';
import { ShieldAlert, Menu, LogOut } from 'lucide-react';
import type { SessionInfo, SkillInfo } from '../types';
import { getTransport } from '../lib/transport';
import { ChatWidget } from './chat/ChatWidget';
import { SessionList } from './SessionList';
import { SkillsCard } from './SkillsCard';
import { CollapsibleCard } from './CollapsibleCard';
import { RoomChatPanel } from './RoomChatPanel';
import { useSessionStore } from '../stores/sessionStore';
import { useServerStore } from '../stores/serverStore';
import { useUserStore } from '../stores/userStore';
export const ConsumerChatPage: React.FC = () => {
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const selectedProjectRoot = useSessionStore((s) => s.selectedProjectRoot);
  const projectStore = useSessionStore.getState();
  const skills = useServerStore((s) => s.skills);
  const userRoomName = useUserStore((s) => s.userRoomName);
  const userTokenBudget = useUserStore((s) => s.userTokenBudget);
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);

  // Skills are already filtered by the server in page_state
  const filteredSkills = skills;

  const handleClickSkill = useCallback((skill: SkillInfo) => {
    if (skill.app) {
      if (skill.app.launcher === 'web') {
        const appUrl = `/apps/${skill.name}/${skill.app.entry}`;
        const instanceId = document.querySelector('meta[name="linggen-instance"]')?.getAttribute('content') || '';
        const relayOrigin = document.querySelector('meta[name="linggen-relay-origin"]')?.getAttribute('content') || '';
        if (instanceId && relayOrigin) {
          window.open(`${relayOrigin}/app/connect/${instanceId}?app=${encodeURIComponent(appUrl)}`, '_blank');
        } else {
          window.open(appUrl, '_blank');
        }
      } else if (skill.app.launcher === 'url') {
        window.open(skill.app.entry, '_blank');
      }
    }
  }, []);

  const handleSelectSession = (session: SessionInfo) => {
    projectStore.setActiveSessionId(session.id);
    projectStore.setIsMissionSession(false);
    const isSkill = session.creator === 'skill' || (!session.project && session.skill);
    projectStore.setIsSkillSession(!!isSkill);
    projectStore.setActiveSkillName(isSkill && session.skill ? session.skill : null);
    const sessionRoot = session.project || session.cwd || session.repo_path || '~';
    if (sessionRoot !== selectedProjectRoot) {
      projectStore.setSelectedProjectRoot(sessionRoot);
    }
    window.localStorage.setItem('linggen:active-session', session.id);
    setMobileMenuOpen(false);
  };

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden">
      {/* Header bar */}
      <header className="flex items-center justify-between px-4 py-2 bg-white dark:bg-[#0f0f0f] border-b border-slate-200 dark:border-white/5 flex-shrink-0">
        <div className="flex items-center gap-3">
          <button
            onClick={() => setMobileMenuOpen(!mobileMenuOpen)}
            className="md:hidden p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500"
          >
            <Menu size={18} />
          </button>
          <div className="flex items-center gap-2">
            <a
              href="https://linggen.dev"
              target="_blank"
              rel="noopener noreferrer"
              className="flex items-center gap-2 hover:opacity-80 transition-opacity"
              title="Open linggen.dev"
            >
              <img src="/linggen-icon.svg" alt="Linggen" className="w-5 h-5" onError={e => { (e.target as HTMLImageElement).style.display = 'none'; }} />
              <span className="text-sm font-bold text-slate-900 dark:text-white">Linggen</span>
            </a>
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-500 font-medium">{userRoomName || 'Proxy Room'}</span>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {/* Privacy indicator */}
          <div className="flex items-center gap-1.5">
            <ShieldAlert size={12} className="text-amber-500" />
            <span className="text-[10px] text-amber-600 dark:text-amber-400 hidden sm:inline">
              Owner can see your messages
            </span>
          </div>
          {userTokenBudget != null && (
            <span className="text-[10px] text-slate-500 hidden sm:inline">
              Budget: {userTokenBudget.toLocaleString()} tokens/day
            </span>
          )}
          <button
            onClick={() => {
              try { getTransport().disconnect(); } catch { /* already gone */ }
              // The consumer page is loaded over the WebRTC tunnel from
              // linggen.dev/app/connect/... — go back to the dashboard.
              // window.close() only works for script-opened popups, so
              // navigate instead. Fall back to close if we're somehow
              // not on linggen.dev (self-hosted instance).
              if (window.location.hostname.endsWith('linggen.dev')) {
                window.location.href = '/app';
              } else {
                window.close();
              }
            }}
            className="flex items-center gap-1 px-2 py-1 rounded text-[11px] font-medium text-red-500 hover:bg-red-500/10 transition-colors"
            title="Leave this room"
          >
            <LogOut size={12} />
            <span className="hidden sm:inline">Leave</span>
          </button>
        </div>
      </header>

      {/* Main layout */}
      <div className="flex-1 flex overflow-hidden">

        {/* Mobile slide-over session list */}
        {mobileMenuOpen && (
          <>
            <div className="fixed inset-0 bg-black/30 z-40 md:hidden" onClick={() => setMobileMenuOpen(false)} />
            <div className="fixed inset-y-0 left-0 w-72 z-50 md:hidden bg-white dark:bg-[#0f0f0f] shadow-xl animate-slide-in flex flex-col">
              <SessionList
                activeSessionId={activeSessionId}
                onSelectSession={handleSelectSession}
                onCreateSession={() => { projectStore.createSession(); setMobileMenuOpen(false); }}
                onDeleteSession={(id) => projectStore.removeSession(id)}
                              />
            </div>
          </>
        )}

        {/* Left sidebar — session list (desktop) */}
        <div className="hidden md:flex w-72 border-r border-slate-200 dark:border-white/5 flex-col bg-white dark:bg-[#0f0f0f] h-full">
          <SessionList
            activeSessionId={activeSessionId}
            onSelectSession={handleSelectSession}
            onCreateSession={() => projectStore.createSession()}
            onDeleteSession={(id) => projectStore.removeSession(id)}
                      />
          <RoomChatPanel />
        </div>

        {/* Center: Chat */}
        <main className="flex-1 flex flex-col overflow-hidden bg-slate-100/40 dark:bg-[#0a0a0a] min-h-0">
          <div className="flex-1 min-h-0 p-2">
            <ChatWidget
              sessionId={activeSessionId}
              projectRoot={selectedProjectRoot}
              mode="full"
            />
          </div>
        </main>

        {/* Right sidebar — allowed skills (desktop) */}
        {filteredSkills.length > 0 && (
          <aside className="hidden lg:flex w-64 border-l border-slate-200 dark:border-white/5 flex-col bg-slate-100/40 dark:bg-[#0a0a0a] p-3 gap-3 overflow-y-auto">
            <CollapsibleCard
              title="SKILLS"
              icon={<span className="text-[10px]">⚡</span>}
              iconColor="text-amber-500"
              badge={`${filteredSkills.length}`}
              defaultOpen
            >
              <SkillsCard skills={filteredSkills} onClickSkill={handleClickSkill} />
            </CollapsibleCard>
          </aside>
        )}
      </div>
    </div>
  );
};
