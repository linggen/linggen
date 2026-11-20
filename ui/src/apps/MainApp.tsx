/**
 * The one shell — owner's full UI: sidebar, chat, info panel, settings, missions.
 * The desktop app's window shows this same shell; what differs there (the
 * tabs, where an app opens, Yinyue in her own window) asks `inShell()`
 * (lib/appHost.ts) and nothing else.
 * Uses responsive CSS for mobile (<768px) — no separate mobile entry.
 */
import React, { useState, useMemo, useEffect, useLayoutEffect, useRef, useCallback } from 'react';
import { chatAgentOf, memberModel } from '../lib/sessionMembers.mts';
import { X } from 'lucide-react';
import type { SessionInfo, SkillInfo } from '../types';
import { appPage, inShell, openApp } from '../lib/appHost';
import { isSettingsTarget } from '../lib/settingsSections.mts';
import { AppFrames, AppTabBar, useAppTabs } from './AppTabs';
import { SkyOverlay } from '../components/SkyOverlay';
import { SessionList } from '../components/SessionList';
import { FilePreview } from '../components/FilePreview';
import { ChatWidget } from '../components/chat';
import { HeaderBar } from '../components/HeaderBar';
import { AgentSpecEditorModal } from '../components/AgentSpecEditorModal';
import { ToastContainer } from '../components/ToastContainer';
import { LingMemConnectModal } from '../components/LingMemConnectModal';
import { YinyueBubble } from '../components/YinyueBubble';
import { YinyueAvatar } from '../components/yinyue/YinyueAvatar';
import { InfoPanel } from '../components/InfoPanel';
import { RoomChatPanel } from '../components/RoomChatPanel';
import { roomsShown } from '../lib/roomsShown.mts';
import { recordSkillUsage } from '../lib/skillUsage';
import { buildAgentWorkInfo } from '../lib/messageUtils';
import { setPresenceCovered, startPresenceBeat } from '../lib/presence';
import { useSessionStore } from '../stores/sessionStore';
import { useServerStore } from '../stores/serverStore';
import { useChatStore } from '../stores/chatStore';
import { useUiStore } from '../stores/uiStore';
import { useUserStore } from '../stores/userStore';
import { useInteractionStore } from '../stores/interactionStore';
import { useRunInfo } from '../hooks/useRunInfo';
import { useChatActions } from '../hooks/useChatActions';
import { sendViewContext, useYinyuePresenter } from '../hooks/useTransport';
import { useOpenSettings } from '../hooks/useOpenSettings';
import '../lib/ownModel';
import { useShallow } from 'zustand/react/shallow';

const urlParams = new URLSearchParams(window.location.search);
const isMobileParam = urlParams.get('mode') === 'mobile';

/** Detect mobile viewport (< 768px) or explicit ?mode=mobile. */
function useIsMobile(): boolean {
  const [mobile, setMobile] = useState(
    isMobileParam || window.innerWidth < 768,
  );
  useEffect(() => {
    if (isMobileParam) return;
    const mq = window.matchMedia('(max-width: 767px)');
    const onChange = (e: MediaQueryListEvent) => setMobile(e.matches);
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, []);
  return mobile;
}

/** Detect remote/tunnel mode (blob iframe with injected instance meta tag). */
const isRemoteMode = typeof document !== 'undefined' && !!document.querySelector('meta[name="linggen-instance"]');

export const MainApp: React.FC<{ covered: boolean }> = ({ covered }) => {
  // --- Stores ---
  // Only the fields this shell renders from. A whole-store subscription made
  // every streamed token (chat store) and token-rate tick (server store)
  // re-render the shell and everything under it — sidebar, session list,
  // info panel — not just the streaming row.
  const { selectedProjectRoot, sessions, allSessions, activeSessionId, isMissionSession, isSkillSession } = useSessionStore(useShallow((s) => ({
    selectedProjectRoot: s.selectedProjectRoot,
    sessions: s.sessions,
    allSessions: s.allSessions,
    activeSessionId: s.activeSessionId,
    isMissionSession: s.isMissionSession,
    isSkillSession: s.isSkillSession,
  })));
  const { agents, models, skills, modelsLoaded, skillsLoaded, selectedAgent, agentStatusText, defaultModels, ollamaStatus, reloadingSkills, agentTreesByProject } = useServerStore(useShallow((s) => ({
    agents: s.agents,
    models: s.models,
    skills: s.skills,
    modelsLoaded: s.modelsLoaded,
    skillsLoaded: s.skillsLoaded,
    selectedAgent: s.selectedAgent,
    agentStatusText: s.agentStatusText,
    defaultModels: s.defaultModels,
    ollamaStatus: s.ollamaStatus,
    reloadingSkills: s.reloadingSkills,
    agentTreesByProject: s.agentTreesByProject,
  })));
  const { showAgentSpecEditor, selectedFileContent, selectedFilePath } = useUiStore(useShallow((s) => ({
    showAgentSpecEditor: s.showAgentSpecEditor,
    selectedFileContent: s.selectedFileContent,
    selectedFilePath: s.selectedFilePath,
  })));
  const projectStore = useSessionStore.getState();
  const agentStore = useServerStore.getState();
  const uiStore = useUiStore.getState();

  const shell = inShell();
  const tabs = useAppTabs({ enabled: shell, covered });

  // Yinyue singleton: subscribe this surface to the server's FCFS presenter
  // lock and render her only when this surface holds it. In the desktop app
  // her own window holds her; this window only lends a bodyless stage while
  // the app page in front says it has no body for her.
  const presenting = useYinyuePresenter(shell ? tabs.bodylessStage : true, shell);
  const showYinyue = !shell && presenting;

  const openSettings = useOpenSettings();
  // A chat framed under this page (an embed) opens Settings → Models here
  // (lib/ownModel.ts); the desktop app's Settings… menu and tray post
  // `linggen:show-settings` (optionally with a section) to this window.
  useEffect(() => {
    window.__linggenOpenSettings = (tab) => openSettings(tab);
    const onMsg = (e: MessageEvent) => {
      if (e.data?.type === 'linggen:show-settings' && e.origin === window.location.origin) {
        openSettings(isSettingsTarget(e.data.section) ? e.data.section : undefined);
      }
    };
    window.addEventListener('message', onMsg);
    return () => {
      delete window.__linggenOpenSettings;
      window.removeEventListener('message', onMsg);
    };
  }, [openSettings]);

  // Only the app's own chat can report presence while its tab is in front.
  useLayoutEffect(() => setPresenceCovered(tabs.appInFront), [tabs.appInFront]);
  useEffect(() => startPresenceBeat(), []);

  const isMobile = useIsMobile();
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const [mobileInfoOpen, setMobileInfoOpen] = useState(false);
  // The right sidebar (models, skills, Yinyue) can fold away on desktop to
  // give the chat the width; the choice is remembered.
  const [rightHidden, setRightHidden] = useState(() => window.localStorage.getItem('linggen:right-panel-hidden') === '1');
  const toggleRight = useCallback(() => setRightHidden((hidden) => {
    window.localStorage.setItem('linggen:right-panel-hidden', hidden ? '0' : '1');
    return !hidden;
  }), []);

  const mainAgents = agents;

  // Session-change tracking (for clear-on-switch)
  const prevSessionIdRef = useRef<string | null>(null);

  // --- Derived memos (sidebar only) ---
  const activeModelId = useMemo(() => {
    if (!activeSessionId) return undefined;
    const text = agentStatusText[activeSessionId] || '';
    if (!text || text === 'Idle') return undefined;
    return text.match(/\(([^)]+)\)/)?.[1];
  }, [agentStatusText, activeSessionId]);

  const mainAgentIds = useMemo(() => agents.map((a) => a.name.toLowerCase()), [agents]);

  const agentTree = useMemo(
    () => agentTreesByProject[selectedProjectRoot] || {},
    [agentTreesByProject, selectedProjectRoot],
  );
  // agentWork is derived here for future use by the sidebar; currently not wired.
  useMemo(() => buildAgentWorkInfo(agentTree), [agentTree]);

  // --- Run info ---
  const { runningMainRunIds } = useRunInfo();

  // --- Chat actions ---
  const scrollToBottomNoop = useCallback(() => {}, []);
  const { sendChatMessage } = useChatActions(scrollToBottomNoop, runningMainRunIds);

  // --- Initial data load ---
  // Most data is delivered via server-pushed page_state after WebRTC connects.
  // Only fetch data not covered by page_state, or needed before WebRTC is ready.
  useEffect(() => {
    useServerStore.getState().fetchOllamaStatus();
    useServerStore.getState().fetchSessionTokens();
  }, []);

  // --- React to selected project changes ---
  // The queue belongs to the session, not its folder: a cd mid-turn moves
  // the folder and must not hide what is still queued (a session switch
  // clears it below).
  useEffect(() => {
    if (selectedProjectRoot) {
      useChatStore.getState().fetchSessionState();
      sendViewContext();
      useInteractionStore.getState().setActivePlan(null);
    }
  }, [selectedProjectRoot]);

  // --- React to session changes ---
  useEffect(() => {
    if (selectedProjectRoot || isMissionSession || isSkillSession || activeSessionId) {
      const prev = prevSessionIdRef.current;
      prevSessionIdRef.current = activeSessionId;
      const isSessionAdoption = prev === null && activeSessionId !== null;

      // Switch to the session's message bucket — instant if cached, no clear needed
      const cs = useChatStore.getState();
      cs.setActiveSession(activeSessionId);

      if (!isSessionAdoption) {
        const interaction = useInteractionStore.getState();
        interaction.setQueuedMessages([]);
        interaction.setActivePlan(null);
        interaction.setPendingPlan(null);
        interaction.setPendingPlanAgentId(null);
        const prevSid = prevSessionIdRef.current;
        if (prevSid) {
          useServerStore.getState().setAgentStatusText((prev) => { const n = { ...prev }; delete n[prevSid]; return n; });
        }
      }
      cs.fetchSessionState();
      sendViewContext();
    }
  }, [activeSessionId, selectedProjectRoot, isMissionSession, isSkillSession]);

  // --- Restore persisted session-level model override ---
  // Look up the active session in `allSessions` (unified cross-project list),
  // not `sessions` (project-scoped). For sessions that don't belong to the
  // currently-selected project — skill sessions, mission sessions, sessions
  // from other projects — `sessions` doesn't contain them, so `find` returns
  // undefined and `setSessionModel(null)` runs on every page_state push,
  // silently reverting any model override the user just picked. `allSessions`
  // contains every session the user can see, so the lookup succeeds
  // regardless of which project the active session lives in.
  // The session's members ride along: the picker shows the model the session
  // keeps for the member this chat speaks with.
  useEffect(() => {
    const sess = allSessions.find((s) => s.id === activeSessionId)
      ?? sessions.find((s) => s.id === activeSessionId);
    const members = sess?.agents ?? [];
    useUiStore.getState().setSessionMembers(members);
    useUiStore.getState().setSessionModel(memberModel(members, chatAgentOf(selectedAgent, members)));
  }, [activeSessionId, allSessions, sessions, selectedAgent]);

  // A pick is for one chat: another session speaks with its own default.
  useEffect(() => {
    useServerStore.getState().setSelectedAgent('');
  }, [activeSessionId]);

  // Mission session messages arrive via the same paths as user-chat sessions:
  //   - one fetch when activeSessionId changes (existing useEffect),
  //   - streaming events while the agent runs,
  //   - StateUpdated → fetchSessionState in handleRunOutcome.
  // No 5s polling — that flooded the control channel for no real benefit.

  // --- A pick of an agent that no longer exists falls back to the default ---
  useEffect(() => {
    if (mainAgentIds.length === 0 || !selectedAgent) return;
    if (!mainAgentIds.includes(selectedAgent.toLowerCase())) {
      useServerStore.getState().setSelectedAgent('');
    }
  }, [mainAgentIds, selectedAgent]);

  // --- Mobile: set data attribute for CSS safe-area insets ---
  useEffect(() => {
    if (isMobile) {
      document.documentElement.setAttribute('data-mobile', '');
    } else {
      document.documentElement.removeAttribute('data-mobile');
    }
  }, [isMobile]);

  // --- Info panel props (shared between desktop sidebar and mobile drawer) ---
  const handleClickSkill = useCallback((skill: SkillInfo) => {
    recordSkillUsage(skill.name);
    const page = appPage(skill);
    if (page) openApp({ skill: skill.name, url: page });
    else sendChatMessage(skill.app ? `/${skill.name} --web` : `/${skill.name}`);
  }, [sendChatMessage]);

  const infoPanelProps = {
    models, skills, modelsLoaded, skillsLoaded, agents: mainAgents, activeModelId,
    defaultModels, ollamaStatus, reloadingSkills,
    projectRoot: selectedProjectRoot,
    onToggleDefault: agentStore.toggleDefaultModel,
    onChangeReasoningEffort: agentStore.setReasoningEffort,
    onReloadSkills: () => agentStore.reloadSkills(),
    onOpenSettings: openSettings,
    onClickSkill: handleClickSkill,
  };

  // --- Remote mode: gate rendering until transport is connected ---
  const connectionStatus = useUserStore((s) => s.connectionStatus);
  const roomsOn = useUserStore((s) => s.roomsOn);
  // The live sky shows over the chat home and settings, never over an app's page.
  const sky = tabs.appInFront ? null : <SkyOverlay />;
  if (isRemoteMode && connectionStatus === 'disconnected') {
    return (
      <>
      {sky}
      <div className="h-screen flex items-center justify-center bg-white dark:bg-[#0b0e14]">
        <div className="text-center space-y-4">
          <div className="inline-block animate-spin rounded-full h-10 w-10 border-4 border-slate-200 dark:border-slate-700 border-t-emerald-500" />
          <p className="text-slate-500 dark:text-slate-400 text-sm">Connecting to linggen server…</p>
        </div>
      </div>
      </>
    );
  }

  const selectSession = (session: SessionInfo, closeMenu?: () => void) => {
    // A session bound to a mission is a mission session regardless of who
    // opened it — an attended dream session has creator 'user' and still
    // lives in the mission's world (its transcript is served by the mission
    // state endpoint, its chat turns carry the mission body).
    const isMission = session.creator === 'mission' || !!session.mission_id;
    const isSkill = session.creator === 'skill' || (!session.project && session.skill);
    projectStore.setActiveSessionId(session.id);
    projectStore.setIsMissionSession(isMission);
    projectStore.setIsSkillSession(!!isSkill);
    projectStore.setActiveSkillName(isSkill && session.skill ? session.skill : null);
    projectStore.setActiveMissionId(isMission && session.mission_id ? session.mission_id : null);
    const sessionRoot = session.project || session.cwd || session.repo_path || '~';
    if (sessionRoot !== selectedProjectRoot) {
      projectStore.setSelectedProjectRoot(sessionRoot);
    }
    window.localStorage.setItem('linggen:active-session', session.id);
    closeMenu?.();
  };

  // --- Render ---
  return (
    <>
      {sky}
      <ToastContainer />
      <LingMemConnectModal />
      {showYinyue && <YinyueBubble />}
      <div className={`flex flex-col h-screen bg-(--surface-shell) text-slate-900 dark:text-slate-200 font-sans overflow-hidden${covered ? ' hidden' : ''}`}>
        {/* Header */}
        <HeaderBar
          covered={covered}
          tabs={tabs.enabled ? <AppTabBar t={tabs} /> : undefined}
          onOpenSettings={() => openSettings('general')}
          onToggleMobileMenu={tabs.active ? undefined : () => setMobileMenuOpen(!mobileMenuOpen)}
          onToggleInfoPanel={tabs.active ? undefined : isMobile ? () => setMobileInfoOpen(!mobileInfoOpen) : toggleRight}
          infoPanelOpen={isMobile ? mobileInfoOpen : !rightHidden}
        />

        {/* Main Layout: the chat home (behind an app's tab in the desktop app) */}
        <div className={`flex-1 flex overflow-hidden${tabs.active ? ' hidden' : ''}`}>

          {/* Mobile slide-over session list */}
          {mobileMenuOpen && (
            <>
              <div className="fixed inset-0 bg-black/30 z-40 md:hidden" onClick={() => setMobileMenuOpen(false)} />
              <div className="fixed inset-y-0 left-0 w-72 z-50 md:hidden bg-white dark:bg-[#0f0f0f] shadow-xl animate-slide-in flex flex-col">
                <SessionList
                  activeSessionId={activeSessionId}
                  onSelectSession={(session) => selectSession(session, () => setMobileMenuOpen(false))}
                  onCreateSession={() => { projectStore.createSession(); setMobileMenuOpen(false); }}
                  onCreateClaudeSession={() => { projectStore.createSession('claude_agent_sdk'); setMobileMenuOpen(false); }}
                  onDeleteSession={(id) => projectStore.removeSession(id)}
                  onOpenSettings={(tab) => { openSettings(tab); setMobileMenuOpen(false); }}
                />
              </div>
            </>
          )}

          {/* Left sidebar — unified session list (desktop only) */}
          <div className="hidden md:flex w-72 border-r border-slate-200 dark:border-white/5 flex-col bg-(--surface-panel) h-full">
            <SessionList
              activeSessionId={activeSessionId}
              onSelectSession={(session) => selectSession(session)}
              onCreateSession={() => projectStore.createSession()}
              onCreateClaudeSession={() => projectStore.createSession('claude_agent_sdk')}
              onDeleteSession={(id) => projectStore.removeSession(id)}
              onOpenSettings={openSettings}
            />
            {roomsShown(roomsOn) && <RoomChatPanel />}
          </div>

          {/* Center: Chat */}
          <main className={`flex-1 flex flex-col overflow-hidden bg-(--surface-pane) min-h-0${isMobile ? ' p-0' : ''}`}>
            <div className="flex-1 min-h-0">
              <ChatWidget
                sessionId={activeSessionId}
                projectRoot={selectedProjectRoot}
                mode={isMobile ? 'mobile' : 'full'}
                flush
              />
            </div>
          </main>

          {/* Right sidebar (desktop only) */}
          {/* Shown and hidden by the header's ✦, lit while it shows. */}
          {!isMobile && !rightHidden && (
            <aside className="hidden lg:flex w-72 border-l border-slate-200 dark:border-white/5 flex-col bg-(--surface-pane) overflow-hidden">
              {/* Models + skills scroll above; Yinyue is docked below. */}
              <div className="flex-1 min-h-0 overflow-y-auto p-3 flex flex-col gap-3">
                <InfoPanel {...infoPanelProps} />
              </div>
              {/* Yinyue's dock — reserved space, click her to talk. Shown only
                  when this surface holds the singleton presenter lock (and never
                  in an app shell, where the native pet window owns her). */}
              {showYinyue && (
                <div className="h-72 shrink-0 border-t border-slate-200 dark:border-white/5">
                  <YinyueAvatar />
                </div>
              )}
            </aside>
          )}

          {/* Mobile right drawer (models + skills) */}
          {isMobile && mobileInfoOpen && (
            <>
              <div className="fixed inset-0 bg-black/30 z-40" onClick={() => setMobileInfoOpen(false)} />
              <div className="fixed inset-y-0 right-0 w-72 z-50 bg-white dark:bg-[#0f0f0f] shadow-xl flex flex-col animate-slide-in-right">
                <div className="flex items-center justify-between px-3 py-2 border-b border-slate-200 dark:border-white/10">
                  <span className="text-xs font-bold uppercase tracking-widest text-slate-500">Info</span>
                  <button onClick={() => setMobileInfoOpen(false)} className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400">
                    <X size={16} />
                  </button>
                </div>
                <div className="flex-1 overflow-y-auto p-3 space-y-3">
                  <InfoPanel {...infoPanelProps} />
                </div>
              </div>
            </>
          )}
        </div>

        <AppFrames t={tabs} />

        <FilePreview selectedFilePath={selectedFilePath} selectedFileContent={selectedFileContent} onClose={() => uiStore.closeFilePreview()} />
        <AgentSpecEditorModal open={showAgentSpecEditor} projectRoot={selectedProjectRoot}
          onClose={() => uiStore.setShowAgentSpecEditor(false)}
          onChanged={() => { agentStore.fetchAgents(selectedProjectRoot); useChatStore.getState().fetchSessionState(); }} />

        <style>{`
          .custom-scrollbar { scrollbar-gutter: stable; }
          .custom-scrollbar::-webkit-scrollbar { width: 8px; }
          .custom-scrollbar::-webkit-scrollbar-track { background: rgba(0, 0, 0, 0.04); }
          .custom-scrollbar::-webkit-scrollbar-thumb { background: rgba(59, 130, 246, 0.45); border-radius: 10px; }
          .custom-scrollbar::-webkit-scrollbar-thumb:hover { background: rgba(59, 130, 246, 0.7); }
        `}</style>
      </div>
    </>
  );
};
