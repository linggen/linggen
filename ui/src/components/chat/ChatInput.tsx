import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Send, Square, X, FolderOpen, FileText } from 'lucide-react';
import { cn } from '../../lib/cn';
import { useInteractionStore } from '../../stores/interactionStore';
import { useSessionStore } from '../../stores/sessionStore';
import { MarkdownContent } from './MarkdownContent';
import { TodoPanel } from './TodoPanel';
import { SuggestionRow } from './SuggestionRow';
import { useSuggestionStore } from '../../stores/suggestionStore';
import { normalizeAgentKey } from '../../lib/messageUtils';
import type {
  AgentInfo,
  FileEntry,
  ModelInfo,
  Plan,
  QueuedChatItem,
  SkillInfo,
} from '../../types';
import { sessionApi, workspaceApi } from '../../lib/endpoints';
import { readImageForSend } from '../../lib/imageEncode';
import {
  agentMatches,
  agentMentionLabel,
  completeAgentMention,
  completeFileMention,
  completeLeadingAgentMention,
  leadingAgentMention,
  mentionLanguage,
  mentionInProgress,
} from '../../lib/chatMentions';

export interface ChatInputProps {
  projectRoot?: string | null;
  selectedAgent: string;
  setSelectedAgent: (value: string) => void;
  skills: SkillInfo[];
  agents: AgentInfo[];
  mainAgentIds: string[];
  isRunning?: boolean;
  onSendMessage: (message: string, targetAgent?: string, images?: string[]) => void;
  onCancelAgentRun?: (runId: string) => void | Promise<void>;
  selectedMainRunningRunId?: string;
  activePlan?: Plan | null;
  visibleQueued: QueuedChatItem[];
  /** The session this chat shows. A skill page's embedded chat has its own;
   *  the app's global store is empty there, so the queue's ✕ did nothing. */
  sessionId?: string | null;
  /** The question open in this chat (AskUser), if any. Words typed while it
   *  is open ARE its answer — the "Other" text — never a message queued
   *  behind it (his, 2026-09-23). A page tap takes another path and waits. */
  openQuestion?: import('../../types').PendingAskUser | null;
  onAnswerQuestion?: (questionId: string, answers: import('../../types').AskUserAnswer[]) => void;
  overlay?: string | null;
  onDismissOverlay?: () => void;
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
  modelPickerOpen?: boolean;
  models?: ModelInfo[];
  defaultModels?: string[];
  onSwitchModel?: (modelId: string) => void;
  mobile?: boolean;
}

export const ChatInput: React.FC<ChatInputProps> = ({
  projectRoot,
  selectedAgent,
  setSelectedAgent,
  skills,
  agents,
  mainAgentIds,
  isRunning,
  onSendMessage,
  onCancelAgentRun,
  selectedMainRunningRunId,
  activePlan,
  visibleQueued,
  sessionId,
  openQuestion,
  onAnswerQuestion,
  overlay,
  onDismissOverlay,
  inputRef,
  modelPickerOpen,
  models: modelsList,
  defaultModels: defaultModelsList,
  onSwitchModel,
  mobile,
}) => {
  const [chatInput, setChatInput] = useState('');
  const hintSessionId = useSessionStore((s) => s.activeSessionId);
  const nextPrompt = useSuggestionStore((s) => (hintSessionId ? s.hints[hintSessionId] : undefined));
  // Each attachment keeps an id, so removing one from the middle keeps the
  // other previews' identity (they were keyed by index).
  const [pendingImages, setPendingImages] = useState<{ id: number; data: string }[]>([]);
  const nextImageId = useRef(0);
  const addImage = (data: string) => setPendingImages((prev) => [...prev, { id: nextImageId.current++, data }]);
  // The grey hint, CC-style: desktop only — a phone keyboard has no Tab, so
  // there the hint stays in the row.
  const hint = !mobile && !isRunning && chatInput === '' && pendingImages.length === 0 ? nextPrompt : undefined;
  const [showSkillDropdown, setShowSkillDropdown] = useState(false);
  const [skillFilter, setSkillFilter] = useState('');
  const [showAgentDropdown, setShowAgentDropdown] = useState(false);
  const [agentFilter, setAgentFilter] = useState('');
  const [showFileDropdown, setShowFileDropdown] = useState(false);
  const [fileFilter, setFileFilter] = useState('');
  const [fileBrowsePath, setFileBrowsePath] = useState('');
  const [fileEntries, setFileEntries] = useState<FileEntry[]>([]);
  const [fileEntriesLoading, setFileEntriesLoading] = useState(false);
  const [fileSearchMode, setFileSearchMode] = useState(false);
  // An `@` opening the message may address an agent, so its dropdown offers
  // the main agents above any files.
  const [mentionAtStart, setMentionAtStart] = useState(false);
  const searchTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [selectedSuggestionIndex, setSelectedSuggestionIndex] = useState(0);
  const [activeSkillHint, setActiveSkillHint] = useState<string | null>(null);

  // The agents a person may address — main agents, less any the spec marks
  // `internal` (run by the engine only).
  const mainAgents = useMemo(
    () => agents.filter((a) => !a.internal && mainAgentIds.includes(normalizeAgentKey(a.name))),
    [agents, mainAgentIds],
  );
  const mentionLang = useMemo(() => mentionLanguage(), []);

  const resizeInput = () => {
    if (!inputRef.current) return;
    inputRef.current.style.height = '0px';
    const full = inputRef.current.scrollHeight;
    const next = Math.min(full, 220);
    inputRef.current.style.height = `${next}px`;
    // The box grows to fit what's in it, so its own scrollbar is meaningless
    // until it stops growing. Left on, a placeholder that wraps in a narrow
    // composer draws a scrollbar beside an empty field.
    inputRef.current.style.overflowY = full > 220 ? 'auto' : 'hidden';
  };

  useEffect(() => {
    resizeInput();
    // isRunning changes the button row, which changes the box's width — and a
    // width the height was never re-measured for is a mis-sized box for the
    // rest of the turn.
  }, [chatInput, isRunning, selectedMainRunningRunId]); // eslint-disable-line react-hooks/exhaustive-deps

  const send = () => {
    if (!chatInput.trim() && pendingImages.length === 0) return;
    const userMessage = chatInput.trim();
    const imagesToSend = pendingImages.length > 0 ? pendingImages.map((img) => img.data) : undefined;
    setChatInput('');
    setPendingImages([]);
    setShowSkillDropdown(false);
    setShowAgentDropdown(false);
    setShowFileDropdown(false);
    setActiveSkillHint(null);
    setFileFilter('');
    setFileBrowsePath('');
    setFileEntries([]);

    // `@@agent` moves the chat to that agent; `@agent` / `@银月` addresses it
    // for this message only — the chat stays with whom it was with.
    const mention = leadingAgentMention(userMessage, mainAgents);
    const mentionAgent = mention?.agent;
    if (mention?.sticky) setSelectedAgent(mention.agent);

    const targetAgent = mentionAgent || selectedAgent;
    if (openQuestion && onAnswerQuestion && userMessage && !imagesToSend && !mentionAgent
        && normalizeAgentKey(openQuestion.agentId) === normalizeAgentKey(targetAgent)) {
      onAnswerQuestion(openQuestion.questionId, openQuestion.questions.map((_, i) => ({
        question_index: i, selected: [], custom_text: i === 0 ? userMessage : null,
      })));
      window.setTimeout(resizeInput, 0);
      return;
    }
    onSendMessage(userMessage, targetAgent, imagesToSend);
    window.setTimeout(resizeInput, 0);
  };

  const handlePaste = async (e: React.ClipboardEvent) => {
    const items = e.clipboardData?.items;
    if (!items) return;
    for (const item of Array.from(items)) {
      if (item.type.startsWith('image/')) {
        e.preventDefault();
        const file = item.getAsFile();
        if (file) {
          addImage(await readImageForSend(file));
        }
      }
    }
  };

  const handleDrop = async (e: React.DragEvent) => {
    const files = e.dataTransfer?.files;
    if (!files) return;
    for (const file of Array.from(files)) {
      if (file.type.startsWith('image/')) {
        e.preventDefault();
        addImage(await readImageForSend(file));
      }
    }
  };

  const handleDragOver = (e: React.DragEvent) => {
    if (e.dataTransfer?.types?.includes('Files')) {
      e.preventDefault();
    }
  };

  const fetchFileEntries = useCallback(async (browsePath: string) => {
    if (!projectRoot) return;
    setFileEntriesLoading(true);
    try {
      const entries = await workspaceApi.files<FileEntry>(projectRoot, browsePath);
      entries.sort((a, b) => (a.isDir !== b.isDir ? (a.isDir ? -1 : 1) : a.name.localeCompare(b.name)));
      setFileEntries(entries);
    } catch {
      setFileEntries([]);
    } finally {
      setFileEntriesLoading(false);
    }
  }, [projectRoot]);

  const searchFiles = useCallback((query: string) => {
    if (!projectRoot) return;
    if (searchTimeoutRef.current) clearTimeout(searchTimeoutRef.current);
    searchTimeoutRef.current = setTimeout(async () => {
      setFileEntriesLoading(true);
      try {
        setFileEntries(await workspaceApi.searchFiles<FileEntry>(projectRoot, query));
      } catch {
        setFileEntries([]);
      } finally {
        setFileEntriesLoading(false);
      }
    }, query ? 150 : 0);
  }, [projectRoot]);

  const filteredFileEntries = useMemo(() => {
    // In search mode, entries are already filtered by the backend
    if (fileSearchMode) return fileEntries;
    let entries = fileEntries;
    if (fileFilter) {
      entries = entries.filter((e) => e.name.toLowerCase().includes(fileFilter.toLowerCase()));
    }
    // Hide dotfiles unless filter starts with "."
    if (!fileFilter.startsWith('.')) {
      entries = entries.filter((e) => !e.name.startsWith('.'));
    }
    return entries;
  }, [fileEntries, fileFilter, fileSearchMode]);

  // Agents a leading `@partial` could address (only in search mode — a path
  // with `/` is never an agent's name).
  const leadAgents = useMemo(
    () => (mentionAtStart && fileSearchMode ? mainAgents.filter((a) => agentMatches(a, fileFilter)) : []),
    [mainAgents, mentionAtStart, fileSearchMode, fileFilter],
  );
  // Without a project root (a skill page's chat) there are no files to find:
  // the `@` dropdown is agents only, and closed when none match.
  const fileRows = projectRoot ? filteredFileEntries : [];
  const mentionRowCount = leadAgents.length + fileRows.length;
  const fileDropdownVisible = showFileDropdown && (!!projectRoot || leadAgents.length > 0);

  const closeFileDropdown = () => {
    setShowFileDropdown(false);
    setFileFilter('');
    setFileBrowsePath('');
    setFileSearchMode(false);
  };

  const applyLeadAgent = (agent: AgentInfo) => {
    setChatInput(completeLeadingAgentMention(chatInput, agentMentionLabel(agent, mentionLang)));
    closeFileDropdown();
    inputRef.current?.focus();
  };

  const skillSuggestions = useMemo(() => {
    const suggestions: {
      key: string;
      label: string;
      description?: string;
      skillName?: string;
      argumentHint?: string | null;
      isBuiltin: boolean;
    }[] = [];

    const builtinCommands: [string, string][] = [
      ['/help', 'Show available commands'],
      ['/clear', 'Clear chat context'],
      ['/compact', 'Compact context (summarize old messages)'],
      ['/status', 'Show project status'],
      ['/model', 'Switch default model'],
      ['/mute', "Mute Yinyue's voice on this Mac"],
      ['/unmute', "Turn Yinyue's voice back on"],
      ['/image', 'Attach an image file'],
    ];

    for (const [cmd, desc] of builtinCommands) {
      const name = cmd.slice(1);
      if (skillFilter === '' || name.includes(skillFilter) || desc.toLowerCase().includes(skillFilter)) {
        suggestions.push({ key: `cmd-${name}`, label: cmd, description: desc, isBuiltin: true });
      }
    }

    skills
      .filter((skill) =>
        skill.name.toLowerCase().includes(skillFilter) ||
        skill.description.toLowerCase().includes(skillFilter)
      )
      .forEach((skill) => {
        suggestions.push({
          key: `skill-${skill.name}`,
          label: `/${skill.name}`,
          description: skill.description,
          skillName: skill.name,
          argumentHint: skill.argument_hint,
          isBuiltin: false,
        });
      });

    return suggestions;
  }, [skills, skillFilter]);

  const applySuggestion = useCallback((suggestion: typeof skillSuggestions[number]) => {
    const beforeSlash = chatInput.substring(0, chatInput.lastIndexOf('/'));
    setChatInput(`${beforeSlash}${suggestion.label} `);
    setShowSkillDropdown(false);
    if (!suggestion.isBuiltin && suggestion.argumentHint) {
      setActiveSkillHint(suggestion.argumentHint);
    }
  }, [chatInput]);

  return (
    <>
      {(overlay || modelPickerOpen) && (
        <div className="absolute bottom-[4.5rem] left-2 right-2 z-20 bg-slate-50 dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-[60%] overflow-y-auto">
          <div className="flex items-center justify-between px-3 py-1.5 border-b border-slate-100 dark:border-white/5 text-[11px] text-slate-400 dark:text-slate-500">
            <span>{modelPickerOpen ? 'Select a model' : 'Press Esc to dismiss'}</span>
            <button onClick={onDismissOverlay} className="hover:text-slate-600 dark:hover:text-slate-300 transition-colors">
              <X size={12} />
            </button>
          </div>
          {modelPickerOpen ? (
            <div className="py-1">
              {(modelsList || []).length === 0 ? (
                <div className="px-3 py-2 text-xs text-slate-400 italic">No models configured</div>
              ) : (
                (modelsList || []).map((m) => {
                  const isDefault = (defaultModelsList || []).includes(m.id);
                  return (
                    <button
                      key={m.id}
                      onClick={() => onSwitchModel?.(m.id)}
                      className={cn(
                        'w-full px-3 py-2 text-left text-xs flex items-center gap-2 transition-colors',
                        isDefault
                          ? 'bg-blue-500/10 text-blue-600 dark:text-blue-400'
                          : 'hover:bg-slate-100 dark:hover:bg-white/5 text-slate-700 dark:text-slate-300'
                      )}
                    >
                      <span className="font-mono">{m.id}</span>
                      <span className="text-slate-400 dark:text-slate-500">({m.provider}: {m.model})</span>
                      {isDefault && <span className="ml-auto text-[11px] font-medium uppercase tracking-wider text-blue-500">active</span>}
                    </button>
                  );
                })
              )}
            </div>
          ) : overlay ? (
            <div className="px-3 py-2 text-xs text-slate-700 dark:text-slate-200 markdown-body">
              <MarkdownContent text={overlay} />
            </div>
          ) : null}
        </div>
      )}
      <div className="sticky bottom-0 z-10 p-2 border-t border-slate-200 dark:border-white/5 space-y-2 bg-slate-50 dark:bg-white/[0.02]">
        {visibleQueued.length > 0 && (
          <div className="px-2 py-1.5 text-[12px] rounded-md border border-amber-300/40 bg-amber-50/80 dark:bg-amber-500/10 dark:border-amber-500/20">
            <div className="flex items-center gap-1.5 text-amber-600 dark:text-amber-400 font-medium select-none">
              <span className="w-1.5 h-1.5 rounded-full bg-amber-500 animate-pulse" />
              <span className="flex-1">{visibleQueued.length} message{visibleQueued.length > 1 ? 's' : ''} queued — agent is busy</span>
              <button
                onClick={async () => {
                  // Authoritative drop on the server first. Without this, queued
                  // messages survive the dismiss and fire when the agent frees up.
                  // Guard before the optimistic clear — if any required id is
                  // missing we skip both calls instead of clearing the local
                  // store while leaving the server queue intact.
                  const store = useSessionStore.getState();
                  const selectedProjectRoot = projectRoot || store.selectedProjectRoot;
                  const activeSessionId = sessionId || store.activeSessionId;
                  if (!selectedProjectRoot || !activeSessionId || !selectedAgent) return;
                  useInteractionStore.getState().setQueuedMessages([]);
                  try {
                    await sessionApi.clearQueue(selectedProjectRoot, activeSessionId, selectedAgent);
                  } catch {
                    // Best-effort clear; UI already reflects the empty queue.
                  }
                }}
                className="text-[11px] px-1.5 py-0.5 rounded bg-amber-200/50 dark:bg-amber-500/20 hover:bg-amber-300/60 dark:hover:bg-amber-500/30 transition-colors"
                title="Dismiss queue"
              >
                <X size={10} />
              </button>
            </div>
            <div className="mt-1 space-y-0.5 text-amber-700 dark:text-amber-300/80">
              {visibleQueued.map((item) => (
                <div key={item.id} className="truncate pl-3">
                  {item.preview}
                </div>
              ))}
            </div>
          </div>
        )}
        {activePlan && activePlan.items && activePlan.items.length > 0 && (
          <TodoPanel plan={activePlan} />
        )}
        {pendingImages.length > 0 && (
          <div className="flex gap-1.5 px-2 py-1.5 flex-wrap">
            {pendingImages.map((img, idx) => (
              <div key={img.id} className="relative group">
                <img
                  src={`data:image/png;base64,${img.data}`}
                  alt={`Pending ${idx + 1}`}
                  className="w-16 h-16 object-cover rounded-md border border-slate-200 dark:border-white/10"
                />
                <button
                  onClick={() => setPendingImages(prev => prev.filter((p) => p.id !== img.id))}
                  className="absolute -top-1.5 -right-1.5 w-5 h-5 rounded-full bg-red-500 text-white flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity"
                  title="Remove image"
                >
                  <X size={10} />
                </button>
              </div>
            ))}
          </div>
        )}
        {activeSkillHint && !showSkillDropdown && (() => {
          const skillMatch = chatInput.match(/^\/([a-zA-Z0-9_-]+)\s*/);
          const skillName = skillMatch?.[1] || '';
          const subcommands = activeSkillHint.split('|').map(s => s.trim()).filter(Boolean);
          return (
            <div className="bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-52 overflow-y-auto">
              <div className="px-3 py-1.5 text-[11px] text-slate-500 border-b border-slate-200 dark:border-white/10">
                /{skillName} • Click a command or type it after /{skillName}
              </div>
              {subcommands.map((cmd) => {
                const colonIdx = cmd.indexOf(':');
                const cmdText = colonIdx >= 0 ? cmd.substring(0, colonIdx).trim() : cmd;
                const desc = colonIdx >= 0 ? cmd.substring(colonIdx + 1).trim() : '';
                const cmdName = cmdText.split(/\s/)[0];
                return (
                  <button
                    key={cmd}
                    onClick={() => {
                      setChatInput(`/${skillName} ${cmdName} `);
                      setActiveSkillHint(null);
                      inputRef.current?.focus();
                    }}
                    className="w-full px-3 py-2 text-left text-xs hover:bg-slate-100 dark:hover:bg-white/5 border-b border-slate-200 dark:border-white/5 last:border-none"
                  >
                    <span className="font-mono font-bold text-blue-500">{cmdText}</span>
                    {desc && <span className="text-slate-500 dark:text-slate-400 ml-2">{desc}</span>}
                  </button>
                );
              })}
            </div>
          );
        })()}
        <SuggestionRow
          skills={skills}
          hidden={!!isRunning || chatInput.trim() !== '' || pendingImages.length > 0}
          hinted={!mobile}
          onPick={(text) => onSendMessage(text)}
        />
        <div className="flex gap-2 bg-white dark:bg-black/20 p-1.5 rounded-xl border border-slate-300/80 dark:border-white/10 relative items-end">
          {showSkillDropdown && (
            <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-52 overflow-y-auto z-[70]">
              <div className="px-3 py-2 text-[11px] text-slate-500 border-b border-slate-200 dark:border-white/10">
                Commands &amp; Skills • Type to filter
              </div>
              {skillSuggestions.map((item, idx) => (
                  <button
                    key={item.key}
                    onClick={() => applySuggestion(item)}
                    className={cn(
                      'w-full px-3 py-2 text-left text-xs border-b border-slate-200 dark:border-white/5 last:border-none',
                      idx === selectedSuggestionIndex
                        ? 'bg-blue-500/10 text-blue-600'
                        : 'hover:bg-slate-100 dark:hover:bg-white/5'
                    )}
                  >
                    <div className={cn('font-bold', item.key.startsWith('cmd-') ? 'text-amber-500' : 'text-blue-500')}>{item.label}</div>
                    {item.description && <div className="text-slate-500 text-[11px]">{item.description}</div>}
                  </button>
                ))}
              {skillSuggestions.length === 0 && (
                <div className="p-3 text-[11px] text-slate-500 italic">No matching commands or skills</div>
              )}
            </div>
          )}
          {showAgentDropdown && (
            <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-48 overflow-y-auto z-[70]">
              {mainAgents
                .filter((agent) => agent.name.toLowerCase().includes(agentFilter))
                .map((agent) => (
                  <button
                    key={agent.name}
                    onClick={() => {
                      setChatInput(completeAgentMention(chatInput, agent.name));
                      setShowAgentDropdown(false);
                      setSelectedAgent(agent.name.toLowerCase());
                    }}
                    className="w-full px-3 py-2 text-left hover:bg-slate-100 dark:hover:bg-white/5 text-xs border-b border-slate-200 dark:border-white/5 last:border-none"
                  >
                    <div className="font-bold text-purple-500">@@{agent.name.charAt(0).toUpperCase() + agent.name.slice(1)}</div>
                    <div className="text-slate-500 text-[11px]">{agent.description}</div>
                  </button>
                ))}
            </div>
          )}
          {fileDropdownVisible && (
            <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-56 overflow-y-auto z-[70]">
              {leadAgents.map((agent, idx) => {
                const label = agentMentionLabel(agent, mentionLang);
                const others = [agent.name.charAt(0).toUpperCase() + agent.name.slice(1), ...(agent.aliases ?? [])].filter((n) => n.toLowerCase() !== label.toLowerCase());
                return (
                  <button
                    key={`agent-${agent.name}`}
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => applyLeadAgent(agent)}
                    className={cn(
                      'w-full px-3 py-1.5 text-left hover:bg-slate-100 dark:hover:bg-white/5 text-xs border-b border-slate-200 dark:border-white/5 last:border-none flex items-center gap-2',
                      idx === selectedSuggestionIndex && 'bg-blue-500/10'
                    )}
                  >
                    <span className="w-5 h-5 rounded-full bg-purple-500/15 text-purple-600 dark:text-purple-300 flex items-center justify-center text-[11px] font-semibold shrink-0">
                      {label.charAt(0)}
                    </span>
                    <span className="font-semibold text-purple-600 dark:text-purple-300">@{label}</span>
                    {others.length > 0 && <span className="text-slate-400 text-[11px]">{others.join(' · ')}</span>}
                    <span className="text-slate-500 text-[11px] truncate min-w-0 flex-1">{agent.description}</span>
                  </button>
                );
              })}
              {projectRoot && !fileSearchMode && fileBrowsePath && (
                <div className="px-3 py-1.5 text-[11px] text-slate-500 dark:text-slate-400 border-b border-slate-200 dark:border-white/5 font-mono truncate">
                  {fileBrowsePath}
                </div>
              )}
              {!projectRoot ? null : fileEntriesLoading ? (
                <div className="p-3 text-[11px] text-slate-500 italic">Loading...</div>
              ) : filteredFileEntries.length === 0 ? (
                leadAgents.length === 0 && <div className="p-3 text-[11px] text-slate-500 italic">No matching files</div>
              ) : (
                filteredFileEntries.map((entry) => (
                  <button
                    key={entry.path}
                    onClick={() => {
                      if (entry.isDir) {
                        // Navigate into directory
                        const newPath = fileSearchMode ? entry.path + '/' : fileBrowsePath + entry.name + '/';
                        setChatInput(completeFileMention(chatInput, newPath, false));
                        setFileBrowsePath(newPath);
                        setFileFilter('');
                        setFileSearchMode(false);
                        setSelectedSuggestionIndex(0);
                        fetchFileEntries(newPath);
                      } else {
                        // Complete file mention
                        const fullPath = fileSearchMode ? entry.path : fileBrowsePath + entry.name;
                        setChatInput(completeFileMention(chatInput, fullPath, true));
                        setShowFileDropdown(false);
                        setFileFilter('');
                        setFileBrowsePath('');
                        setFileSearchMode(false);
                      }
                    }}
                    className={cn(
                      'w-full px-3 py-1.5 text-left hover:bg-slate-100 dark:hover:bg-white/5 text-xs border-b border-slate-200 dark:border-white/5 last:border-none flex items-center gap-2',
                      leadAgents.length + filteredFileEntries.indexOf(entry) === selectedSuggestionIndex && 'bg-blue-500/10'
                    )}
                  >
                    {entry.isDir ? (
                      <FolderOpen size={13} className="text-amber-500 shrink-0" />
                    ) : (
                      <FileText size={13} className="text-slate-400 shrink-0" />
                    )}
                    {fileSearchMode ? (
                      <span className="font-mono truncate">{entry.path}</span>
                    ) : (
                      <span className={entry.isDir ? 'font-semibold' : ''}>{entry.name}</span>
                    )}
                  </button>
                ))
              )}
            </div>
          )}
          <textarea
            ref={inputRef}
            value={chatInput}
            onPaste={handlePaste}
            onDrop={handleDrop}
            onDragOver={handleDragOver}
            onChange={(e) => {
              const val = e.target.value;
              setChatInput(val);

              // Skill dropdown: `/` trigger
              if (val.includes('/') && !val.includes(' ', val.lastIndexOf('/'))) {
                const lastSlash = val.lastIndexOf('/');
                // Only trigger skill dropdown if the `/` is not part of a file path (i.e. after `@`)
                const atIdx = val.lastIndexOf('@');
                if (atIdx < 0 || lastSlash < atIdx) {
                  setSkillFilter(val.substring(lastSlash + 1).toLowerCase());
                  setShowSkillDropdown(true);
                  setShowAgentDropdown(false);
                  setShowFileDropdown(false);
                  setSelectedSuggestionIndex(0);
                  return;
                }
              }

              const mention = mentionInProgress(val);
              if (mention) {
                setShowAgentDropdown(mention.kind === 'agent');
                setShowFileDropdown(mention.kind !== 'agent');
                setShowSkillDropdown(false);
                setSelectedSuggestionIndex(0);
                if (mention.kind === 'agent') {
                  setAgentFilter(mention.filter);
                } else if (mention.kind === 'file-browse') {
                  setFileFilter(mention.filter);
                  setFileSearchMode(false);
                  if (mention.dir !== fileBrowsePath || fileEntries.length === 0) {
                    setFileBrowsePath(mention.dir);
                    fetchFileEntries(mention.dir);
                  }
                } else {
                  setFileFilter(mention.query);
                  setFileSearchMode(true);
                  setFileBrowsePath('');
                  searchFiles(mention.query);
                }
                setMentionAtStart(mention.kind === 'file-search' && mention.atStart);
                return;
              }

              // No dropdown — but check for skill argument hint
              setShowSkillDropdown(false);
              setShowAgentDropdown(false);
              setShowFileDropdown(false);

              // Show argument hint when user typed "/skillname " (skill + space, no subcommand yet)
              const skillMatch = val.match(/^\/([a-zA-Z0-9_-]+)(\s*)(.*)$/);
              if (skillMatch && skillMatch[2]) {
                const afterSkill = skillMatch[3].trim();
                if (!afterSkill) {
                  // Just "/skillname " — show hint
                  const matched = skills.find(
                    (s) => s.name.toLowerCase() === skillMatch[1].toLowerCase() && s.argument_hint
                  );
                  setActiveSkillHint(matched?.argument_hint ?? null);
                } else {
                  // User started typing subcommand — hide hint
                  setActiveSkillHint(null);
                }
              } else {
                setActiveSkillHint(null);
              }
            }}
            onKeyDown={(e) => {
              // Ignore Enter during IME composition (e.g. Chinese pinyin input)
              if (e.key === 'Enter' && (e.nativeEvent.isComposing || e.keyCode === 229)) return;
              // Tab takes the grey hint into the input; Enter then sends it.
              if (e.key === 'Tab' && !e.shiftKey && hint) {
                e.preventDefault();
                setChatInput(hint);
                return;
              }
              // Skill dropdown keyboard nav
              if (showSkillDropdown && skillSuggestions.length > 0) {
                if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                  e.preventDefault();
                  const delta = e.key === 'ArrowDown' ? 1 : -1;
                  setSelectedSuggestionIndex((prev) => (prev + delta + skillSuggestions.length) % skillSuggestions.length);
                  return;
                }
                if (e.key === 'Enter') {
                  const exactCommands = ['/help', '/clear', '/compact', '/status', '/model'];
                  if (exactCommands.includes(chatInput.trim())) {
                    e.preventDefault();
                    setShowSkillDropdown(false);
                    send();
                    return;
                  }
                  e.preventDefault();
                  const selected = skillSuggestions[selectedSuggestionIndex];
                  if (selected) applySuggestion(selected);
                  return;
                }
              }

              // `@` dropdown keyboard nav: agents (leading `@` only), then files
              if (fileDropdownVisible && mentionRowCount > 0) {
                if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                  e.preventDefault();
                  const delta = e.key === 'ArrowDown' ? 1 : -1;
                  setSelectedSuggestionIndex((prev) => (prev + delta + mentionRowCount) % mentionRowCount);
                  return;
                }
                const agent = leadAgents[selectedSuggestionIndex];
                if (agent && (e.key === 'Enter' || e.key === 'Tab')) {
                  e.preventDefault();
                  applyLeadAgent(agent);
                  return;
                }
                const entry = fileRows[selectedSuggestionIndex - leadAgents.length];
                if (e.key === 'Enter' || (e.key === 'Tab' && entry?.isDir)) {
                  e.preventDefault();
                  if (!entry) return;
                  const lastAt = chatInput.lastIndexOf('@');
                  const beforeAt = chatInput.substring(0, lastAt);
                  if (entry.isDir) {
                    const newPath = fileSearchMode ? entry.path + '/' : fileBrowsePath + entry.name + '/';
                    setChatInput(`${beforeAt}@${newPath}`);
                    setFileBrowsePath(newPath);
                    setFileFilter('');
                    setFileSearchMode(false);
                    setSelectedSuggestionIndex(0);
                    fetchFileEntries(newPath);
                  } else {
                    const fullPath = fileSearchMode ? entry.path : fileBrowsePath + entry.name;
                    setChatInput(`${beforeAt}@${fullPath} `);
                    setShowFileDropdown(false);
                    setFileFilter('');
                    setFileBrowsePath('');
                    setFileSearchMode(false);
                  }
                  return;
                }
              }

              // Agent dropdown keyboard nav
              if (showAgentDropdown) {
                const filteredAgents = mainAgents.filter((a) => a.name.toLowerCase().includes(agentFilter));
                if (filteredAgents.length > 0) {
                  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                    e.preventDefault();
                    const delta = e.key === 'ArrowDown' ? 1 : -1;
                    setSelectedSuggestionIndex((prev) => (prev + delta + filteredAgents.length) % filteredAgents.length);
                    return;
                  }
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    const agent = filteredAgents[selectedSuggestionIndex];
                    if (!agent) return;
                    setChatInput(completeAgentMention(chatInput, agent.name));
                    setShowAgentDropdown(false);
                    setSelectedAgent(agent.name.toLowerCase());
                    return;
                  }
                }
              }

              // Send on Enter (only when no dropdown open)
              if (e.key === 'Enter' && !e.shiftKey && !showSkillDropdown && !showAgentDropdown && !fileDropdownVisible) {
                e.preventDefault();
                send();
              }
              if (e.key === 'Escape') {
                if (overlay && onDismissOverlay) {
                  onDismissOverlay();
                }
                setShowSkillDropdown(false);
                setShowAgentDropdown(false);
                setShowFileDropdown(false);
              }
            }}
            placeholder={hint ? `${hint}   ⇥ Tab` : mobile ? "Message..." : "Message... (/ for skills, @ for files, Shift+Enter for newline)"}
            rows={1}
            className={cn(
              "flex-1 bg-transparent border-none outline-none resize-none leading-5 overflow-y-hidden",
              mobile ? "px-2 py-2.5 text-[16px] min-h-[42px] max-h-[160px]" : "px-1.5 py-1.5 text-[14px] min-h-[34px] max-h-[200px]",
            )}
          />
          {isRunning && selectedMainRunningRunId && onCancelAgentRun && (
            <button
              onClick={() => onCancelAgentRun(selectedMainRunningRunId)}
              className={cn(
                "rounded-lg bg-red-500 text-white flex items-center justify-center hover:bg-red-600 transition-colors",
                mobile ? "w-10 h-10" : "w-8 h-8",
              )}
              title="Stop agent"
            >
              <Square size={mobile ? 14 : 12} fill="currentColor" />
            </button>
          )}
          <button
            onClick={send}
            className={cn(
              "rounded-lg bg-blue-600 text-white flex items-center justify-center hover:bg-blue-500 transition-colors",
              mobile ? "w-10 h-10" : "w-8 h-8",
            )}
            title="Send"
          >
            <Send size={mobile ? 16 : 14} />
          </button>
        </div>
      </div>
    </>
  );
};
