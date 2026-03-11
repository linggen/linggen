import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Send, Square, X, FolderOpen, FileText } from 'lucide-react';
import { cn } from '../../lib/cn';
import { MarkdownContent } from './MarkdownContent';
import { TodoPanel } from './TodoPanel';
import { normalizeAgentKey } from './utils/message';
import type {
  AgentInfo,
  FileEntry,
  ModelInfo,
  Plan,
  QueuedChatItem,
  SkillInfo,
} from '../../types';

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
  overlay?: string | null;
  onDismissOverlay?: () => void;
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
  modelPickerOpen?: boolean;
  models?: ModelInfo[];
  defaultModels?: string[];
  onSwitchModel?: (modelId: string) => void;
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
  overlay,
  onDismissOverlay,
  inputRef,
  modelPickerOpen,
  models: modelsList,
  defaultModels: defaultModelsList,
  onSwitchModel,
}) => {
  const [chatInput, setChatInput] = useState('');
  const [pendingImages, setPendingImages] = useState<string[]>([]);
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
  const searchTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [selectedSuggestionIndex, setSelectedSuggestionIndex] = useState(0);

  const resizeInput = () => {
    if (!inputRef.current) return;
    inputRef.current.style.height = '0px';
    const next = Math.min(inputRef.current.scrollHeight, 220);
    inputRef.current.style.height = `${next}px`;
  };

  useEffect(() => {
    resizeInput();
  }, [chatInput]); // eslint-disable-line react-hooks/exhaustive-deps

  const send = () => {
    if (!chatInput.trim() && pendingImages.length === 0) return;
    const userMessage = chatInput.trim();
    const imagesToSend = pendingImages.length > 0 ? [...pendingImages] : undefined;
    setChatInput('');
    setPendingImages([]);
    setShowSkillDropdown(false);
    setShowAgentDropdown(false);
    setShowFileDropdown(false);
    setFileFilter('');
    setFileBrowsePath('');
    setFileEntries([]);

    const mentionMatch = userMessage.trim().match(/^@@([a-zA-Z0-9_-]+)\b/);
    let mentionAgent: string | undefined;
    if (mentionMatch?.[1]) {
      const mentioned = normalizeAgentKey(mentionMatch[1]);
      if (mainAgentIds.includes(mentioned)) {
        mentionAgent = mentioned;
        setSelectedAgent(mentioned);
      }
    }

    const targetAgent = mentionAgent || selectedAgent;
    onSendMessage(userMessage, targetAgent, imagesToSend);
    window.setTimeout(resizeInput, 0);
  };

  const readFileAsBase64 = (file: File): Promise<string> => {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const result = reader.result as string;
        const base64 = result.split(',')[1] || result;
        resolve(base64);
      };
      reader.onerror = reject;
      reader.readAsDataURL(file);
    });
  };

  const handlePaste = async (e: React.ClipboardEvent) => {
    const items = e.clipboardData?.items;
    if (!items) return;
    for (const item of Array.from(items)) {
      if (item.type.startsWith('image/')) {
        e.preventDefault();
        const file = item.getAsFile();
        if (file) {
          const base64 = await readFileAsBase64(file);
          setPendingImages(prev => [...prev, base64]);
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
        const base64 = await readFileAsBase64(file);
        setPendingImages(prev => [...prev, base64]);
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
      const url = `/api/files?project_root=${encodeURIComponent(projectRoot)}&path=${encodeURIComponent(browsePath)}`;
      const resp = await fetch(url);
      if (resp.ok) {
        const entries: FileEntry[] = await resp.json();
        entries.sort((a, b) => (a.isDir !== b.isDir ? (a.isDir ? -1 : 1) : a.name.localeCompare(b.name)));
        setFileEntries(entries);
      }
    } catch {
      setFileEntries([]);
    } finally {
      setFileEntriesLoading(false);
    }
  }, [projectRoot]);

  const searchFiles = useCallback((query: string) => {
    if (!projectRoot) return;
    if (searchTimeoutRef.current) clearTimeout(searchTimeoutRef.current);
    if (!query) {
      setFileEntries([]);
      return;
    }
    searchTimeoutRef.current = setTimeout(async () => {
      setFileEntriesLoading(true);
      try {
        const url = `/api/files/search?project_root=${encodeURIComponent(projectRoot)}&query=${encodeURIComponent(query)}`;
        const resp = await fetch(url);
        if (resp.ok) setFileEntries(await resp.json());
      } catch {
        setFileEntries([]);
      } finally {
        setFileEntriesLoading(false);
      }
    }, 150);
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

  const buildSkillSuggestions = () => {
    const suggestions: {
      key: string;
      label: string;
      description?: string;
      apply: () => void;
    }[] = [];

    const beforeSlash = chatInput.substring(0, chatInput.lastIndexOf('/'));

    // Built-in commands (aligned with TUI)
    const builtinCommands: [string, string][] = [
      ['/help', 'Show available commands'],
      ['/clear', 'Clear chat context'],
      ['/compact', 'Compact context (summarize old messages)'],
      ['/status', 'Show project status'],
      ['/model', 'Switch default model'],
      ['/image', 'Attach an image file'],
    ];

    for (const [cmd, desc] of builtinCommands) {
      const name = cmd.slice(1); // strip leading /
      if (skillFilter === '' || name.includes(skillFilter) || desc.toLowerCase().includes(skillFilter)) {
        suggestions.push({
          key: `cmd-${name}`,
          label: cmd,
          description: desc,
          apply: () => {
            setChatInput(`${beforeSlash}${cmd} `);
            setShowSkillDropdown(false);
          },
        });
      }
    }

    // Skills from API
    skills
      .filter(
        (skill) =>
          skill.name.toLowerCase().includes(skillFilter) ||
          skill.description.toLowerCase().includes(skillFilter)
      )
      .forEach((skill) => {
        suggestions.push({
          key: `skill-${skill.name}`,
          label: `/${skill.name}`,
          description: skill.description,
          apply: () => {
            setChatInput(`${beforeSlash}/${skill.name} `);
            setShowSkillDropdown(false);
          },
        });
      });

    return suggestions;
  };

  return (
    <>
      {(overlay || modelPickerOpen) && (
        <div className="absolute bottom-[4.5rem] left-2 right-2 z-20 bg-slate-50 dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-[60%] overflow-y-auto">
          <div className="flex items-center justify-between px-3 py-1.5 border-b border-slate-100 dark:border-white/5 text-[10px] text-slate-400 dark:text-slate-500">
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
                      {isDefault && <span className="ml-auto text-[10px] font-medium uppercase tracking-wider text-blue-500">active</span>}
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
          <div className="px-2 py-1.5 text-[11px] rounded-md border border-amber-300/40 bg-amber-50/80 dark:bg-amber-500/10 dark:border-amber-500/20">
            <div className="flex items-center gap-1.5 text-amber-600 dark:text-amber-400 font-medium select-none">
              <span className="w-1.5 h-1.5 rounded-full bg-amber-500 animate-pulse" />
              {visibleQueued.length} message{visibleQueued.length > 1 ? 's' : ''} queued — agent is busy
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
              <div key={idx} className="relative group">
                <img
                  src={`data:image/png;base64,${img}`}
                  alt={`Pending ${idx + 1}`}
                  className="w-16 h-16 object-cover rounded-md border border-slate-200 dark:border-white/10"
                />
                <button
                  onClick={() => setPendingImages(prev => prev.filter((_, i) => i !== idx))}
                  className="absolute -top-1.5 -right-1.5 w-5 h-5 rounded-full bg-red-500 text-white flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity"
                  title="Remove image"
                >
                  <X size={10} />
                </button>
              </div>
            ))}
          </div>
        )}
        <div className="flex gap-2 bg-white dark:bg-black/20 p-1.5 rounded-xl border border-slate-300/80 dark:border-white/10 relative items-end">
          {showSkillDropdown && (
            <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-52 overflow-y-auto z-[70]">
              <div className="px-3 py-2 text-[10px] text-slate-500 border-b border-slate-200 dark:border-white/10">
                Commands &amp; Skills • Type to filter
              </div>
              {(() => {
                const suggestions = buildSkillSuggestions();
                return suggestions.map((item, idx) => (
                  <button
                    key={item.key}
                    onClick={item.apply}
                    className={cn(
                      'w-full px-3 py-2 text-left text-xs border-b border-slate-200 dark:border-white/5 last:border-none',
                      idx === selectedSuggestionIndex
                        ? 'bg-blue-500/10 text-blue-600'
                        : 'hover:bg-slate-100 dark:hover:bg-white/5'
                    )}
                  >
                    <div className={cn('font-bold', item.key.startsWith('cmd-') ? 'text-amber-500' : 'text-blue-500')}>{item.label}</div>
                    {item.description && <div className="text-slate-500 text-[10px]">{item.description}</div>}
                  </button>
                ));
              })()}
              {buildSkillSuggestions().length === 0 && (
                <div className="p-3 text-[10px] text-slate-500 italic">No matching commands or skills</div>
              )}
            </div>
          )}
          {showAgentDropdown && (
            <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-48 overflow-y-auto z-[70]">
              {agents
                .filter((agent) => mainAgentIds.includes(normalizeAgentKey(agent.name)))
                .filter((agent) => agent.name.toLowerCase().includes(agentFilter))
                .map((agent) => (
                  <button
                    key={agent.name}
                    onClick={() => {
                      const doubleAtIdx = chatInput.lastIndexOf('@@');
                      const beforeAt = doubleAtIdx >= 0 ? chatInput.substring(0, doubleAtIdx) : chatInput;
                      const label = agent.name.charAt(0).toUpperCase() + agent.name.slice(1);
                      setChatInput(`${beforeAt}@@${label} `);
                      setShowAgentDropdown(false);
                      setSelectedAgent(agent.name.toLowerCase());
                    }}
                    className="w-full px-3 py-2 text-left hover:bg-slate-100 dark:hover:bg-white/5 text-xs border-b border-slate-200 dark:border-white/5 last:border-none"
                  >
                    <div className="font-bold text-purple-500">@@{agent.name.charAt(0).toUpperCase() + agent.name.slice(1)}</div>
                    <div className="text-slate-500 text-[10px]">{agent.description}</div>
                  </button>
                ))}
            </div>
          )}
          {showFileDropdown && (
            <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-56 overflow-y-auto z-[70]">
              {!fileSearchMode && fileBrowsePath && (
                <div className="px-3 py-1.5 text-[10px] text-slate-500 dark:text-slate-400 border-b border-slate-200 dark:border-white/5 font-mono truncate">
                  {fileBrowsePath}
                </div>
              )}
              {fileEntriesLoading ? (
                <div className="p-3 text-[10px] text-slate-500 italic">Loading...</div>
              ) : filteredFileEntries.length === 0 ? (
                <div className="p-3 text-[10px] text-slate-500 italic">No matching files</div>
              ) : (
                filteredFileEntries.map((entry) => (
                  <button
                    key={entry.path}
                    onClick={() => {
                      const lastAt = chatInput.lastIndexOf('@');
                      const beforeAt = chatInput.substring(0, lastAt);
                      if (entry.isDir) {
                        // Navigate into directory
                        const newPath = fileSearchMode ? entry.path + '/' : fileBrowsePath + entry.name + '/';
                        setChatInput(`${beforeAt}@${newPath}`);
                        setFileBrowsePath(newPath);
                        setFileFilter('');
                        setFileSearchMode(false);
                        setSelectedSuggestionIndex(0);
                        fetchFileEntries(newPath);
                      } else {
                        // Complete file mention
                        const fullPath = fileSearchMode ? entry.path : fileBrowsePath + entry.name;
                        setChatInput(`${beforeAt}@${fullPath} `);
                        setShowFileDropdown(false);
                        setFileFilter('');
                        setFileBrowsePath('');
                        setFileSearchMode(false);
                      }
                    }}
                    className={cn(
                      'w-full px-3 py-1.5 text-left hover:bg-slate-100 dark:hover:bg-white/5 text-xs border-b border-slate-200 dark:border-white/5 last:border-none flex items-center gap-2',
                      filteredFileEntries.indexOf(entry) === selectedSuggestionIndex && 'bg-blue-500/10'
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

              // Find last `@` not preceded by a space-after check
              const lastAt = val.lastIndexOf('@');
              if (lastAt >= 0 && !val.includes(' ', lastAt)) {
                // Check for `@@` (agent mention)
                if (lastAt > 0 && val[lastAt - 1] === '@') {
                  const afterDoubleAt = val.substring(lastAt + 1);
                  // Only show agent dropdown if no space after @@
                  if (!afterDoubleAt.includes(' ')) {
                    setAgentFilter(afterDoubleAt.toLowerCase());
                    setShowAgentDropdown(true);
                    setShowSkillDropdown(false);
                    setShowFileDropdown(false);
                    setSelectedSuggestionIndex(0);
                    return;
                  }
                }

                // Single `@` — file mention
                const afterAt = val.substring(lastAt + 1);
                // Don't trigger file dropdown if next char is also `@`
                if (!afterAt.startsWith('@') && !afterAt.includes(' ')) {
                  const pathText = afterAt;
                  const lastSlashInPath = pathText.lastIndexOf('/');

                  setShowFileDropdown(true);
                  setShowAgentDropdown(false);
                  setShowSkillDropdown(false);
                  setSelectedSuggestionIndex(0);

                  if (lastSlashInPath >= 0) {
                    // Has `/` → directory browse mode
                    const browsePath = pathText.substring(0, lastSlashInPath + 1);
                    const filterText = pathText.substring(lastSlashInPath + 1);
                    setFileFilter(filterText);
                    setFileSearchMode(false);
                    if (browsePath !== fileBrowsePath || fileEntries.length === 0) {
                      setFileBrowsePath(browsePath);
                      fetchFileEntries(browsePath);
                    }
                  } else {
                    // No `/` → recursive search mode
                    setFileFilter(pathText);
                    setFileSearchMode(true);
                    setFileBrowsePath('');
                    searchFiles(pathText);
                  }
                  return;
                }
              }

              // No dropdown
              setShowSkillDropdown(false);
              setShowAgentDropdown(false);
              setShowFileDropdown(false);
            }}
            onKeyDown={(e) => {
              // Skill dropdown keyboard nav
              const suggestions = showSkillDropdown ? buildSkillSuggestions() : [];
              if (showSkillDropdown && suggestions.length > 0) {
                if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                  e.preventDefault();
                  const delta = e.key === 'ArrowDown' ? 1 : -1;
                  setSelectedSuggestionIndex((prev) => (prev + delta + suggestions.length) % suggestions.length);
                  return;
                }
                if (e.key === 'Enter') {
                  // If the input exactly matches a complete built-in command, send it directly
                  const exactCommands = ['/help', '/clear', '/compact', '/status', '/model'];
                  if (exactCommands.includes(chatInput.trim())) {
                    e.preventDefault();
                    setShowSkillDropdown(false);
                    send();
                    return;
                  }
                  e.preventDefault();
                  suggestions[selectedSuggestionIndex]?.apply();
                  return;
                }
              }

              // File dropdown keyboard nav
              if (showFileDropdown && filteredFileEntries.length > 0) {
                if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                  e.preventDefault();
                  const delta = e.key === 'ArrowDown' ? 1 : -1;
                  setSelectedSuggestionIndex((prev) => (prev + delta + filteredFileEntries.length) % filteredFileEntries.length);
                  return;
                }
                if (e.key === 'Enter' || (e.key === 'Tab' && filteredFileEntries[selectedSuggestionIndex]?.isDir)) {
                  e.preventDefault();
                  const entry = filteredFileEntries[selectedSuggestionIndex];
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
                const filteredAgents = agents
                  .filter((a) => mainAgentIds.includes(normalizeAgentKey(a.name)))
                  .filter((a) => a.name.toLowerCase().includes(agentFilter));
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
                    const doubleAtIdx = chatInput.lastIndexOf('@@');
                    const beforeAt = doubleAtIdx >= 0 ? chatInput.substring(0, doubleAtIdx) : chatInput;
                    const label = agent.name.charAt(0).toUpperCase() + agent.name.slice(1);
                    setChatInput(`${beforeAt}@@${label} `);
                    setShowAgentDropdown(false);
                    setSelectedAgent(agent.name.toLowerCase());
                    return;
                  }
                }
              }

              // Send on Enter (only when no dropdown open)
              if (e.key === 'Enter' && !e.shiftKey && !showSkillDropdown && !showAgentDropdown && !showFileDropdown) {
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
            placeholder="Message... (/ for skills, @ for files, @@ for agents, Shift+Enter for newline)"
            rows={1}
            className="flex-1 bg-transparent border-none px-1.5 py-1.5 text-[13px] outline-none resize-none min-h-[34px] max-h-[200px] leading-5"
          />
          {isRunning && selectedMainRunningRunId && onCancelAgentRun && (
            <button
              onClick={() => onCancelAgentRun(selectedMainRunningRunId)}
              className="w-8 h-8 rounded-lg bg-red-500 text-white flex items-center justify-center hover:bg-red-600 transition-colors"
              title="Stop agent"
            >
              <Square size={12} fill="currentColor" />
            </button>
          )}
          <button
            onClick={send}
            className="w-8 h-8 rounded-lg bg-blue-600 text-white flex items-center justify-center hover:bg-blue-500 transition-colors"
            title="Send"
          >
            <Send size={14} />
          </button>
        </div>
      </div>
    </>
  );
};
