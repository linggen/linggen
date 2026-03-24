import React, { useState, useMemo, useEffect, useRef, useCallback } from 'react';
import { Plus, Settings, X } from 'lucide-react';
import { cn } from './lib/cn';
import { SessionList } from './components/SessionList';
import { FilePreview } from './components/FilePreview';
import { ChatWidget } from './components/chat';
import { HeaderBar } from './components/HeaderBar';
import { SettingsPage } from './components/SettingsPage';
import { MissionEditor } from './components/MissionPage';
import { AgentSpecEditorModal } from './components/AgentSpecEditorModal';
import { ToastContainer } from './components/ToastContainer';
import { AppPanel } from './components/AppPanel';
import { InfoPanel } from './components/InfoPanel';
import {
  buildAgentWorkInfo,
} from './lib/messageUtils';
import { useProjectStore } from './stores/projectStore';
import { useAgentStore } from './stores/agentStore';
import { useChatStore } from './stores/chatStore';
import { useUiStore } from './stores/uiStore';
import { useRunInfo } from './hooks/useRunInfo';
import { useChatActions } from './hooks/useChatActions';

// ---------------------------------------------------------------------------
// Compact mode (VS Code sidebar)
// ---------------------------------------------------------------------------

const compactParams = new URLSearchParams(window.location.search);
const isCompact = compactParams.get('mode') === 'compact';
const isMobileParam = compactParams.get('mode') === 'mobile';
const compactProject = compactParams.get('project') || '';
const compactSession = compactParams.get('session') || '';
const compactHideToolbar = compactParams.get('hide_toolbar') === '1';

/** Detect mobile viewport (< 768px) or explicit ?mode=mobile. */
function useIsMobile(): boolean {
  const [mobile, setMobile] = useState(
    isMobileParam || (!isCompact && window.innerWidth < 768),
  );
  useEffect(() => {
    if (isMobileParam || isCompact) return; // explicit mode — no resize detection
    const mq = window.matchMedia('(max-width: 767px)');
    const onChange = (e: MediaQueryListEvent) => setMobile(e.matches);
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, []);
  return mobile;
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

/** Detect remote/tunnel mode (blob iframe with injected instance meta tag). */
const isRemoteMode = typeof document !== 'undefined' && !!document.querySelector('meta[name="linggen-instance"]');

const App: React.FC = () => {
  // --- Stores ---
  const projectStore = useProjectStore();
  const agentStore = useAgentStore();
  const chatStore = useChatStore();
  const uiStore = useUiStore();

  const isMobile = useIsMobile();
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const [mobileInfoOpen, setMobileInfoOpen] = useState(false);

  // Shortcuts
  const { projects, selectedProjectRoot, sessions, activeSessionId, isMissionSession, agentTreesByProject } = projectStore;
  const { agents, models, skills, selectedAgent, agentStatus, agentStatusText, agentContext, defaultModels, ollamaStatus, sessionTokens, tokensPerSec, reloadingSkills, reloadingAgents } = agentStore;
  const { messages: chatMessages } = chatStore;
  const { currentPage, editingMission, missionRefreshKey, showAgentSpecEditor, openApp, selectedFileContent, selectedFilePath, copyChatStatus } = uiStore;

  const isRunning = agentStore.isRunning();
  const mainAgents = agents;

  // Session-change tracking (for clear-on-switch)
  const prevSessionIdRef = useRef<string | null>(null);

  // --- Derived memos (sidebar only) ---
  const activeModelId = useMemo(() => {
    for (const name of Object.keys(agentStatusText)) {
      const status = agentStatus[name];
      if (status && status !== 'idle') {
        const text = agentStatusText[name] || '';
        const match = text.match(/\(([^)]+)\)/);
        if (match) return match[1];
      }
    }
    return undefined;
  }, [agentStatus, agentStatusText]);

  const mainAgentIds = useMemo(() => agents.map((a) => a.name.toLowerCase()), [agents]);

  const agentTree = useMemo(
    () => agentTreesByProject[selectedProjectRoot] || {},
    [agentTreesByProject, selectedProjectRoot],
  );
  const agentWork = useMemo(() => buildAgentWorkInfo(agentTree), [agentTree]);

  // --- Run info (for sidebar AgentsCard) ---
  const { agentRunSummary, runningMainRunIds } = useRunInfo();

  // --- Chat actions (for header clear/copy) ---
  const scrollToBottomNoop = useCallback(() => {}, []);
  const { clearChat, copyChat, sendChatMessage } = useChatActions(scrollToBottomNoop, runningMainRunIds);

  // --- Initial data load ---
  useEffect(() => {
    // In compact skill mode, only fetch models (for the model picker).
    // Skip projects, sessions, skills, etc. — the iframe is a clean slate.
    if (isCompact && compactSession) {
      useAgentStore.getState().fetchModels();
      useAgentStore.getState().fetchSkills();
      useAgentStore.getState().fetchAgents();
      return;
    }
    useProjectStore.getState().fetchProjects();
    useProjectStore.getState().fetchAllSessions();
    useAgentStore.getState().fetchSkills();
    useAgentStore.getState().fetchAgents();
    useAgentStore.getState().fetchModels();
    useAgentStore.getState().fetchDefaultModels();

    const interval = setInterval(() => { useAgentStore.getState().fetchOllamaStatus(); useAgentStore.getState().fetchSessionTokens(); }, 5000);
    useAgentStore.getState().fetchOllamaStatus();
    useAgentStore.getState().fetchSessionTokens();
    return () => clearInterval(interval);
  }, []);

  // --- React to selected project changes ---
  useEffect(() => {
    // In compact skill mode, skip all project-related fetching — the iframe
    // only needs the skill session, not the project's sessions/files/agents.
    if (isCompact && compactSession) return;
    if (selectedProjectRoot) {
      useProjectStore.getState().fetchFiles();
      useChatStore.getState().fetchWorkspaceState();
      useProjectStore.getState().fetchAgentTree(selectedProjectRoot);
      useAgentStore.getState().fetchAgentRuns();
      useProjectStore.getState().fetchSessions();
      useAgentStore.getState().fetchAgents(selectedProjectRoot);
      useAgentStore.getState().resetStatus();
      useUiStore.getState().setQueuedMessages([]);
      useUiStore.getState().setActivePlan(null);
    }
  }, [selectedProjectRoot]);

  // --- React to projects list changes ---
  useEffect(() => {
    if (isCompact && compactSession) return;
    if (projects.length > 0) {
      useProjectStore.getState().fetchAllAgentTrees();
      useProjectStore.getState().fetchAllSessionCounts();
    }
  }, [projects]);

  const fetchPendingAskUser = useCallback(() => {
    useUiStore.getState().fetchPendingAskUser();
  }, []);

  // --- React to session changes ---
  useEffect(() => {
    if (selectedProjectRoot || isMissionSession) {
      const prev = prevSessionIdRef.current;
      prevSessionIdRef.current = activeSessionId;
      const isSessionAdoption = prev === null && activeSessionId !== null;

      // Switch to the session's message bucket — instant if cached, no clear needed
      const chatStore = useChatStore.getState();
      chatStore.setActiveSession(activeSessionId);

      if (!isSessionAdoption) {
        const ui = useUiStore.getState();
        ui.setQueuedMessages([]);
        ui.setActivePlan(null);
        ui.setPendingPlan(null);
        ui.setPendingPlanAgentId(null);
        ui.setSessionModel(null);
      }
      // Always fetch to merge latest server state into the bucket
      chatStore.fetchWorkspaceState();
      useAgentStore.getState().fetchAgentRuns();
      // Restore any pending AskUser/permission widget for this session.
      fetchPendingAskUser();
    }
  }, [activeSessionId, selectedProjectRoot, isMissionSession, fetchPendingAskUser]);

  // --- Poll workspace state for mission sessions (backup; SSE also triggers reloads) ---
  useEffect(() => {
    if (!isMissionSession || !activeSessionId) return;
    const interval = setInterval(() => {
      useChatStore.getState().fetchWorkspaceState();
    }, 5000);
    return () => clearInterval(interval);
  }, [isMissionSession, activeSessionId]);

  // --- Auto-select agent ---
  useEffect(() => {
    if (mainAgentIds.length === 0) return;
    if (!mainAgentIds.includes(selectedAgent.toLowerCase())) {
      const preferred = mainAgentIds.includes('ling') ? 'ling' : mainAgentIds[0];
      useAgentStore.getState().setSelectedAgent(preferred);
    }
  }, [mainAgentIds, selectedAgent]);

  // --- Compact mode init ---
  const compactSessionInitRef = useRef(false);
  useEffect(() => {
    if (!isCompact || compactSessionInitRef.current) return;
    compactSessionInitRef.current = true;

    // Skill-bound embed mode: session is created by the SDK wrapper.
    // Clear selectedProjectRoot in memory (not localStorage) so API calls
    // don't inherit the main page's project — the backend will use cwd.
    if (compactSession) {
      useProjectStore.setState({ activeSessionId: compactSession, selectedProjectRoot: '' });
      return;
    }

    // VS Code compact mode: auto-create/resume project session
    if (!compactProject) return;
    useProjectStore.getState().setSelectedProjectRoot(compactProject);
    (async () => {
      try {
        const resp = await fetch(`/api/sessions?project_root=${encodeURIComponent(compactProject)}`);
        const data = await resp.json();
        const sessionList = data.sessions ?? data ?? [];
        const existing = sessionList.find((s: any) => s.title?.startsWith('VS Code'));
        if (existing) {
          useProjectStore.getState().setActiveSessionId(existing.id);
        } else {
          const createResp = await fetch('/api/sessions', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ project_root: compactProject, title: 'VS Code' }),
          });
          const created = await createResp.json();
          useProjectStore.getState().setActiveSessionId(created.id);
        }
      } catch (e) {
        console.error('Compact session init error:', e);
      }
    })();
  }, []);

  // --- Mobile: set data attribute for CSS safe-area insets ---
  useEffect(() => {
    if (isMobile) {
      document.documentElement.setAttribute('data-mobile', '');
    } else {
      document.documentElement.removeAttribute('data-mobile');
    }
  }, [isMobile]);

  // --- Clipboard bridge for VS Code compact mode ---
  const clearChatRef = useRef(clearChat);
  const sendChatMessageRef = useRef(sendChatMessage);
  useEffect(() => { clearChatRef.current = clearChat; sendChatMessageRef.current = sendChatMessage; }, [clearChat, sendChatMessage]);
  useEffect(() => {
    if (!isCompact) return;
    document.documentElement.setAttribute('data-compact', '');
    const handleMessage = (e: MessageEvent) => {
      if (e.data?.type !== 'linggen-clipboard') return;
      const sel = window.getSelection();
      const text = sel?.toString();
      switch (e.data.action) {
        case 'copy': if (text) navigator.clipboard.writeText(text).catch(() => {}); break;
        case 'cut': if (text) { navigator.clipboard.writeText(text).catch(() => {}); document.execCommand('delete'); } break;
        case 'selectAll': document.execCommand('selectAll'); break;
      }
    };
    const handleCopy = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 'c' && !e.shiftKey && !e.altKey) {
        const sel = window.getSelection();
        const text = sel?.toString();
        if (text) { e.preventDefault(); navigator.clipboard.writeText(text).catch(() => {}); }
      }
    };
    // Skill app bridge: parent page sends commands via postMessage when
    // this chat panel is embedded as an iframe in an app skill page.
    const handleSkillCommand = (e: MessageEvent) => {
      if (e.data?.type !== 'linggen-skill') return;
      const { action, payload } = e.data;
      switch (action) {
        case 'send': {
          // Send directly via the chat action — no DOM manipulation needed.
          sendChatMessageRef.current(payload?.text || '');
          break;
        }
        case 'add_message':
          useChatStore.getState().addMessage({
            role: payload?.role === 'user' ? 'user' as const : 'agent' as const,
            from: payload?.role === 'user' ? 'user' : 'assistant',
            to: '',
            text: payload?.text || '',
            timestamp: new Date().toLocaleTimeString(),
            timestampMs: Date.now(),
            isGenerating: false,
          });
          break;
        case 'clear':
          clearChatRef.current();
          break;
      }
    };
    window.addEventListener('message', handleSkillCommand);

    window.addEventListener('message', handleMessage);
    document.addEventListener('keydown', handleCopy);
    return () => {
      window.removeEventListener('message', handleSkillCommand);
      window.removeEventListener('message', handleMessage);
      document.removeEventListener('keydown', handleCopy);
    };
  }, []);

  // --- Sidebar callbacks ---
  const readFile = useCallback(async (path: string, projectRootOverride?: string) => {
    const root = projectRootOverride || useProjectStore.getState().selectedProjectRoot;
    if (!root) return;
    try {
      const resp = await fetch(`/api/file?project_root=${encodeURIComponent(root)}&path=${encodeURIComponent(path)}`);
      const data = await resp.json();
      useUiStore.getState().setSelectedFileContent(data.content);
      useUiStore.getState().setSelectedFilePath(path);
    } catch (e) { console.error('Error reading file:', e); }
  }, []);

  const selectAgentPathFromTree = useCallback((projectRoot: string, path: string) => {
    if (projectRoot !== useProjectStore.getState().selectedProjectRoot) useProjectStore.getState().setSelectedProjectRoot(projectRoot);
    readFile(path, projectRoot);
  }, [readFile]);

  // --- Info panel props (shared between desktop sidebar and mobile drawer) ---
  const handleClickSkill = useCallback((skill: any) => {
    if (skill.app) {
      if (skill.app.launcher === 'web') window.open(`/apps/${skill.name}/${skill.app.entry}`, '_blank');
      else if (skill.app.launcher === 'url') window.open(skill.app.entry, '_blank');
      else sendChatMessage(`/${skill.name} --web`);
    } else sendChatMessage(`/${skill.name}`);
  }, [sendChatMessage]);

  const infoPanelProps = {
    models, skills, agents: mainAgents, chatMessages, tokensPerSec, activeModelId,
    agentContext, defaultModels, ollamaStatus, sessionTokens, reloadingSkills,
    projectRoot: selectedProjectRoot,
    onToggleDefault: agentStore.toggleDefaultModel,
    onChangeReasoningEffort: agentStore.setReasoningEffort,
    onReloadSkills: () => agentStore.reloadSkills(),
    onOpenSettings: (tab: string) => uiStore.openSettings(tab),
    onClickSkill: handleClickSkill,
  };

  // --- Remote mode: gate rendering until transport is connected ---
  if (isRemoteMode && uiStore.connectionStatus === 'disconnected') {
    return (
      <div className="h-screen flex items-center justify-center bg-white dark:bg-[#0b0e14]">
        <div className="text-center space-y-4">
          <div className="inline-block animate-spin rounded-full h-10 w-10 border-4 border-slate-200 dark:border-slate-700 border-t-emerald-500" />
          <p className="text-slate-500 dark:text-slate-400 text-sm">Connecting to linggen server…</p>
        </div>
      </div>
    );
  }

  // --- Render ---
  return (
    <>
    <ToastContainer />
    {!isCompact && currentPage === 'mission-editor' && (
      <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden">
        <MissionEditor
          editing={editingMission}
          projects={projects}
          onSave={() => { uiStore.bumpMissionRefreshKey(); uiStore.closeMissionEditor(); }}
          onCancel={() => uiStore.closeMissionEditor()}
          onViewAgent={() => {}}
        />
      </div>
    )}
    {!isCompact && currentPage === 'settings' && (
      <SettingsPage
        onBack={() => {
          uiStore.setCurrentPage('main');
          uiStore.setInitialSettingsTab(undefined);
          agentStore.fetchModels();
          agentStore.fetchDefaultModels();
          agentStore.fetchOllamaStatus();
        }}
        projectRoot={selectedProjectRoot}
        initialTab={uiStore.initialSettingsTab}
        missionAgents={agents}
      />
    )}
    <div className={`flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden${currentPage !== 'main' ? ' hidden' : ''}`}>
      {/* Header */}
      {!isCompact && (
        <HeaderBar
          copyChat={copyChat}
          copyChatStatus={copyChatStatus}
          clearChat={clearChat}
          isRunning={isRunning}
          onOpenSettings={() => uiStore.setCurrentPage('settings')}
          onToggleMobileMenu={() => setMobileMenuOpen(!mobileMenuOpen)}
          onToggleInfoPanel={isMobile ? () => setMobileInfoOpen(!mobileInfoOpen) : undefined}
        />
      )}

      {/* Compact toolbar (hidden when embedded by SDK) */}
      {isCompact && !compactHideToolbar && (
        <div className="flex items-center gap-1.5 px-2 py-1 border-b border-slate-200 dark:border-white/10 bg-white dark:bg-[#0f0f0f] flex-shrink-0">
          <select value={selectedAgent} onChange={(e) => agentStore.setSelectedAgent(e.target.value)}
            className="text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 text-slate-700 dark:text-slate-300 outline-none max-w-[5rem]">
            {agents.map((a) => <option key={a.name} value={a.name}>{a.name}</option>)}
          </select>
          <select value={activeSessionId || ''} onChange={(e) => { projectStore.setActiveSessionId(e.target.value || null); projectStore.setIsMissionSession(false); }}
            className="text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 text-slate-700 dark:text-slate-300 outline-none flex-1 min-w-0 truncate">
            {sessions.length === 0 && <option value="">No sessions</option>}
            {sessions.map((s) => <option key={s.id} value={s.id}>{s.title || s.id.slice(0, 8)}</option>)}
          </select>
          <button onClick={() => projectStore.createSession()} title="New chat session"
            className="p-0.5 rounded hover:bg-slate-200 dark:hover:bg-white/10 text-slate-500 hover:text-slate-700 dark:hover:text-slate-300 transition-colors flex-shrink-0">
            <Plus size={14} />
          </button>
          <span className={`text-[10px] flex-shrink-0 ${isRunning ? 'text-green-500' : 'text-slate-400'}`}>
            {isRunning ? 'Running' : 'Idle'}
          </span>
        </div>
      )}

      {/* Main Layout */}
      <div className="flex-1 flex overflow-hidden">

        {/* Mobile slide-over session list */}
        {!isCompact && mobileMenuOpen && (
          <>
            <div className="fixed inset-0 bg-black/30 z-40 md:hidden" onClick={() => setMobileMenuOpen(false)} />
            <div className="fixed inset-y-0 left-0 w-72 z-50 md:hidden bg-white dark:bg-[#0f0f0f] shadow-xl animate-slide-in flex flex-col">
              <SessionList
                activeSessionId={activeSessionId}
                onSelectSession={(session) => {
                  const isMission = session.creator === 'mission';
                  projectStore.setActiveSessionId(session.id);
                  projectStore.setIsMissionSession(isMission);
                  projectStore.setActiveMissionId(isMission && session.mission_id ? session.mission_id : null);
                  if (session.project && session.project !== selectedProjectRoot) {
                    projectStore.setSelectedProjectRoot(session.project);
                  }
                  window.localStorage.setItem('linggen:active-session', session.id);
                  setMobileMenuOpen(false);
                }}
                onCreateSession={() => { projectStore.createSession(); setMobileMenuOpen(false); }}
                onDeleteSession={(id) => projectStore.removeSession(id)}
                onOpenSettings={(tab) => { uiStore.setCurrentPage('settings'); setMobileMenuOpen(false); }}
              />
            </div>
          </>
        )}

        {/* Left sidebar — unified session list (desktop only) */}
        {!isCompact && (
        <div className="hidden md:flex w-72 border-r border-slate-200 dark:border-white/5 flex-col bg-white dark:bg-[#0f0f0f] h-full">
          <SessionList
            activeSessionId={activeSessionId}
            onSelectSession={(session) => {
              const isMission = session.creator === 'mission';
              projectStore.setActiveSessionId(session.id);
              projectStore.setIsMissionSession(isMission);
              if (isMission && session.mission_id) {
                projectStore.setActiveMissionId(session.mission_id);
              } else {
                projectStore.setActiveMissionId(null);
              }
              // Switch project context if session belongs to a different project
              if (session.project && session.project !== selectedProjectRoot) {
                projectStore.setSelectedProjectRoot(session.project);
              }
              window.localStorage.setItem('linggen:active-session', session.id);
            }}
            onCreateSession={() => projectStore.createSession()}
            onDeleteSession={(id) => projectStore.removeSession(id)}
            onOpenSettings={(tab) => uiStore.setCurrentPage('settings')}
          />
        </div>
        )}

        {/* Center: Chat */}
        <main className={`flex-1 flex flex-col overflow-hidden bg-slate-100/40 dark:bg-[#0a0a0a] min-h-0${isCompact || isMobile ? ' p-0' : ''}`}>
          <div className={`flex-1 min-h-0${isCompact || isMobile ? '' : ' p-2'}`}>
            <ChatWidget
              sessionId={activeSessionId}
              projectRoot={selectedProjectRoot}
              mode={isCompact ? 'compact' : isMobile ? 'mobile' : 'full'}
            />
          </div>
        </main>

        {/* Right sidebar (desktop only) */}
        {!isCompact && !isMobile && (
        <aside className="hidden lg:flex w-72 border-l border-slate-200 dark:border-white/5 flex-col bg-slate-100/40 dark:bg-[#0a0a0a] p-3 gap-3 overflow-y-auto">
          <InfoPanel {...infoPanelProps} />
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

      <FilePreview selectedFilePath={selectedFilePath} selectedFileContent={selectedFileContent} onClose={() => uiStore.closeFilePreview()} />
      <AgentSpecEditorModal open={showAgentSpecEditor} projectRoot={selectedProjectRoot}
        onClose={() => uiStore.setShowAgentSpecEditor(false)}
        onChanged={() => { agentStore.fetchAgents(selectedProjectRoot); chatStore.fetchWorkspaceState(); projectStore.fetchAllAgentTrees(); }} />

      {openApp && <AppPanel app={openApp} onClose={() => uiStore.setOpenApp(null)} />}

      <style>{`
        .custom-scrollbar { scrollbar-gutter: stable; }
        .custom-scrollbar::-webkit-scrollbar { width: 8px; }
        .custom-scrollbar::-webkit-scrollbar-track { background: rgba(0, 0, 0, 0.04); }
        .custom-scrollbar::-webkit-scrollbar-thumb { background: rgba(59, 130, 246, 0.45); border-radius: 10px; }
        .custom-scrollbar::-webkit-scrollbar-thumb:hover { background: rgba(59, 130, 246, 0.7); }
        html[data-compact] { --vsc-bg: #1e1e1e; --vsc-sidebar: #252526; --vsc-input: #3c3c3c; --vsc-border: #3c3c3c; --vsc-fg: #cccccc; --vsc-fg-muted: #858585; --vsc-accent: #0e639c; color-scheme: dark; }
        html[data-compact] .dark\\:bg-\\[\\#0a0a0a\\] { background-color: var(--vsc-bg) !important; }
        html[data-compact] .dark\\:bg-\\[\\#0f0f0f\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:bg-white\\/\\[0\\.02\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:bg-white\\/5 { background-color: rgba(255,255,255,0.03) !important; }
        html[data-compact] .dark\\:bg-black\\/20 { background-color: var(--vsc-input) !important; }
        html[data-compact] .dark\\:bg-black\\/30 { background-color: var(--vsc-input) !important; }
        html[data-compact] .dark\\:border-white\\/5, html[data-compact] .dark\\:border-white\\/10 { border-color: var(--vsc-border) !important; }
        html[data-compact] section { border-radius: 0 !important; border: none !important; }
        html[data-compact] .dark\\:text-slate-200 { color: var(--vsc-fg) !important; }
        html[data-compact] .dark\\:text-slate-300 { color: #d4d4d4 !important; }
        html[data-compact] .dark\\:text-slate-400 { color: #969696 !important; }
        html[data-compact] .text-slate-500 { color: #969696 !important; }
        html[data-compact] .text-slate-400 { color: #969696 !important; }
        html[data-compact] .text-slate-600 { color: #b0b0b0 !important; }
        html[data-compact] select, html[data-compact] input, html[data-compact] textarea { color: var(--vsc-fg) !important; background-color: var(--vsc-input) !important; border-color: var(--vsc-border) !important; font-size: 13px !important; }
        html[data-compact] ::placeholder { color: var(--vsc-fg-muted) !important; opacity: 1 !important; }
        html[data-compact] body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; font-size: 13px; color: var(--vsc-fg); }
        html[data-compact] .custom-scrollbar::-webkit-scrollbar-thumb { background: rgba(121,121,121,0.4); border-radius: 0; }
        html[data-compact] .custom-scrollbar::-webkit-scrollbar-thumb:hover { background: rgba(121,121,121,0.7); }
        html[data-compact] .custom-scrollbar::-webkit-scrollbar-track { background: transparent; }
        html[data-compact] .dark\\:bg-\\[\\#141414\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:border-amber-500\\/20, html[data-compact] .dark\\:border-amber-500\\/10 { border-color: var(--vsc-border) !important; }
        html[data-compact] .dark\\:bg-amber-500\\/5 { background-color: rgba(255,255,255,0.03) !important; }
        html[data-compact] .dark\\:bg-amber-500\\/10 { background-color: rgba(255,255,255,0.05) !important; }
        html[data-compact] .dark\\:text-amber-400 { color: #cca700 !important; }
        html[data-compact] .dark\\:text-amber-300 { color: #ddb700 !important; }
        html[data-compact] .dark\\:text-amber-300\\/80 { color: rgba(221,183,0,0.8) !important; }
        html[data-compact] .dark\\:hover\\:border-amber-500\\/40:hover { border-color: #cca700 !important; }
        html[data-compact] .dark\\:hover\\:bg-amber-500\\/5:hover { background-color: rgba(255,255,255,0.05) !important; }
        html[data-compact] .dark\\:border-amber-500\\/30 { border-color: rgba(204,167,0,0.4) !important; }
        html[data-compact] .dark\\:border-blue-500\\/20, html[data-compact] .dark\\:border-blue-500\\/10 { border-color: var(--vsc-border) !important; }
        html[data-compact] .dark\\:bg-blue-500\\/5 { background-color: rgba(255,255,255,0.03) !important; }
        html[data-compact] .dark\\:text-blue-400 { color: #569cd6 !important; }
        html[data-compact] .dark\\:hover\\:border-blue-500\\/40:hover { border-color: #569cd6 !important; }
        html[data-compact] .dark\\:bg-blue-500 { background-color: var(--vsc-accent) !important; border-color: var(--vsc-accent) !important; }
        html[data-compact] .dark\\:hover\\:bg-blue-600:hover { background-color: #1177bb !important; }
        html[data-compact] .dark\\:bg-white\\/5 { background-color: var(--vsc-input) !important; border-color: var(--vsc-border) !important; }
        html[data-compact] .dark\\:hover\\:bg-white\\/10:hover { background-color: rgba(255,255,255,0.08) !important; }
        html[data-compact] .dark\\:hover\\:border-red-500\\/40:hover { border-color: #f14c4c !important; }
        html[data-compact] .dark\\:hover\\:text-red-400:hover { color: #f14c4c !important; }
        html[data-compact] .dark\\:hover\\:border-slate-400\\/30:hover { border-color: var(--vsc-fg-muted) !important; }
        html[data-compact] .dark\\:hover\\:text-slate-400:hover { color: #b0b0b0 !important; }
        html[data-compact] .dark\\:hover\\:text-slate-300:hover { color: #d4d4d4 !important; }
        html[data-compact] .dark\\:hover\\:text-amber-400:hover { color: #cca700 !important; }
        html[data-compact] .dark\\:hover\\:border-amber-500\\/30:hover { border-color: rgba(204,167,0,0.4) !important; }
        html[data-compact] .dark\\:bg-amber-600 { background-color: #b8860b !important; }
        html[data-compact] .dark\\:hover\\:bg-amber-700:hover { background-color: #996f0a !important; }
        html[data-compact] section { border-radius: 2px !important; border: 1px solid var(--vsc-border) !important; }
        html[data-compact] .rounded-xl { border-radius: 2px !important; }
        html[data-compact] .rounded-lg { border-radius: 2px !important; }
        html[data-compact] .rounded-md { border-radius: 2px !important; }
        html[data-compact] .shadow-sm { box-shadow: none !important; }
        html[data-compact] .shadow-xl { box-shadow: 0 2px 8px rgba(0,0,0,0.36) !important; }
        html[data-compact] .dark\\:text-green-300 { color: #89d185 !important; }
        html[data-compact] .dark\\:text-blue-300 { color: #569cd6 !important; }
        html[data-compact] .dark\\:text-amber-300 { color: #cca700 !important; }
        html[data-compact] .dark\\:text-indigo-300 { color: #b4a0ff !important; }
        html[data-compact] .dark\\:text-slate-600 { color: #5a5a5a !important; }
        html[data-compact] .dark\\:text-slate-500 { color: #6e6e6e !important; }
        html[data-compact] .dark\\:focus\\:border-amber-500\\/40:focus { border-color: var(--vsc-accent) !important; }
        html[data-compact] .dark\\:focus\\:border-blue-500\\/40:focus { border-color: var(--vsc-accent) !important; }
        html[data-compact] .dark\\:bg-white\\/\\[0\\.02\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:hover\\:bg-white\\/5:hover { background-color: rgba(255,255,255,0.06) !important; }
        html[data-compact] .markdown-body { color: var(--vsc-fg, #ccc) !important; }
        html[data-compact] .markdown-body code { background: rgba(255,255,255,0.1) !important; color: #d4d4d4 !important; }
        html[data-compact] .markdown-body pre { background: rgba(255,255,255,0.06) !important; }
        html[data-compact] .markdown-body pre code { color: #c9d1d9 !important; }
        html[data-compact] .markdown-body th, html[data-compact] .markdown-body td { border-color: rgba(255,255,255,0.15) !important; color: #d4d4d4 !important; }
        html[data-compact] .markdown-body th { background: rgba(255,255,255,0.06) !important; }
        html[data-compact] .markdown-body hr { border-top-color: rgba(255,255,255,0.12) !important; }
        html[data-compact] .markdown-body blockquote { border-left-color: rgba(59,130,246,0.5) !important; color: #b4becd !important; }
        html[data-compact] .markdown-body h1, html[data-compact] .markdown-body h2, html[data-compact] .markdown-body h3, html[data-compact] .markdown-body h4 { color: #e0e0e0 !important; }
        html[data-compact] .hljs { color: #c9d1d9 !important; background: transparent !important; }
        html[data-compact] .hljs-doctag, html[data-compact] .hljs-keyword, html[data-compact] .hljs-template-tag, html[data-compact] .hljs-template-variable, html[data-compact] .hljs-type, html[data-compact] .hljs-variable.language_ { color: #ff7b72 !important; }
        html[data-compact] .hljs-title, html[data-compact] .hljs-title.class_, html[data-compact] .hljs-title.function_ { color: #d2a8ff !important; }
        html[data-compact] .hljs-attr, html[data-compact] .hljs-attribute, html[data-compact] .hljs-literal, html[data-compact] .hljs-meta, html[data-compact] .hljs-number, html[data-compact] .hljs-operator, html[data-compact] .hljs-variable, html[data-compact] .hljs-selector-attr, html[data-compact] .hljs-selector-class, html[data-compact] .hljs-selector-id { color: #79c0ff !important; }
        html[data-compact] .hljs-regexp, html[data-compact] .hljs-string { color: #a5d6ff !important; }
        html[data-compact] .hljs-built_in, html[data-compact] .hljs-symbol { color: #ffa657 !important; }
        html[data-compact] .hljs-comment, html[data-compact] .hljs-code, html[data-compact] .hljs-formula { color: #8b949e !important; }
        html[data-compact] .hljs-name, html[data-compact] .hljs-quote, html[data-compact] .hljs-selector-tag, html[data-compact] .hljs-selector-pseudo { color: #7ee787 !important; }
        html[data-compact] .hljs-subst { color: #c9d1d9 !important; }
        html[data-compact] .hljs-addition { color: #aff5b4 !important; background-color: #033a16 !important; }
        html[data-compact] .hljs-deletion { color: #ffdcd7 !important; background-color: #67060c !important; }
      `}</style>
    </div>
    </>
  );
};

export default App;
