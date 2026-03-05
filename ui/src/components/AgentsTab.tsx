import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { FilePlus2, FileText, Save, Trash2 } from 'lucide-react';
import type { AgentFileInfo } from '../types';
import { CM6Editor } from './CM6Editor';

const defaultAgentTemplate = (agentName: string) => `---
name: ${agentName}
description: ${agentName} agent.
tools: [Read]
model: inherit
kind: main
work_globs: ["**/*"]
policy: []
---

You are linggen '${agentName}'.
`;

const policyBadges: { key: string; label: string; cls: string }[] = [
  { key: 'Patch', label: 'P', cls: 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-400' },
  { key: 'Finalize', label: 'F', cls: 'bg-violet-500/10 text-violet-700 dark:text-violet-400' },
  { key: 'Delegate', label: 'D', cls: 'bg-amber-500/10 text-amber-700 dark:text-amber-400' },
];

// Kind badges removed — all agents are equal.

const parsePolicyFromContent = (content: string): string[] => {
  const match = content.match(/^policy:\s*\[([^\]]*)\]/m);
  if (!match) return [];
  return match[1]
    .split(',')
    .map((s) => s.trim().replace(/['"]/g, ''))
    .filter(Boolean);
};

const parseDelegateTargets = (content: string): string => {
  const match = content.match(/^delegate_targets:\s*\[([^\]]*)\]/m);
  if (!match) return '';
  return match[1]
    .split(',')
    .map((s) => s.trim().replace(/['"]/g, ''))
    .filter(Boolean)
    .join(', ');
};

const updatePolicyInContent = (content: string, policies: string[]): string => {
  const policyStr = policies.length > 0 ? `[${policies.join(', ')}]` : '[]';
  if (content.match(/^policy:\s*\[/m)) {
    return content.replace(/^policy:\s*\[[^\]]*\]/m, `policy: ${policyStr}`);
  }
  // Insert before closing ---
  const parts = content.split('---');
  if (parts.length >= 3) {
    const frontmatter = parts[1].trimEnd();
    return `---${frontmatter}\npolicy: ${policyStr}\n---${parts.slice(2).join('---')}`;
  }
  return content;
};

const updateDelegateTargetsInContent = (content: string, targets: string): string => {
  const arr = targets
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
  const val = arr.length > 0 ? `[${arr.join(', ')}]` : '[]';
  if (content.match(/^delegate_targets:\s*\[/m)) {
    return content.replace(/^delegate_targets:\s*\[[^\]]*\]/m, `delegate_targets: ${val}`);
  }
  if (!targets.trim()) return content;
  const parts = content.split('---');
  if (parts.length >= 3) {
    const frontmatter = parts[1].trimEnd();
    return `---${frontmatter}\ndelegate_targets: ${val}\n---${parts.slice(2).join('---')}`;
  }
  return content;
};

export const AgentsTab: React.FC<{
  projectRoot: string;
  onChanged?: () => void;
}> = ({ projectRoot, onChanged }) => {
  const [files, setFiles] = useState<AgentFileInfo[]>([]);
  const [selectedPath, setSelectedPath] = useState<string>('');
  const [content, setContent] = useState<string>('');
  const [savedContent, setSavedContent] = useState<string>('');
  const [loadingList, setLoadingList] = useState(false);
  const [loadingFile, setLoadingFile] = useState(false);
  const [saving, setSaving] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);

  const dirty = useMemo(() => content !== savedContent, [content, savedContent]);

  const currentPolicies = useMemo(() => parsePolicyFromContent(content), [content]);
  const delegateTargets = useMemo(() => parseDelegateTargets(content), [content]);

  const fetchList = useCallback(async () => {
    if (!projectRoot) return;
    setLoadingList(true);
    try {
      const resp = await fetch(`/api/agent-files?project_root=${encodeURIComponent(projectRoot)}`);
      if (!resp.ok) return;
      const data = (await resp.json()) as AgentFileInfo[];
      setFiles(data);
      if (data.length === 0) {
        setSelectedPath('');
        setContent('');
        setSavedContent('');
        return;
      }
      setSelectedPath((prev) =>
        !prev || !data.some((item) => item.path === prev) ? data[0].path : prev
      );
    } finally {
      setLoadingList(false);
    }
  }, [projectRoot]);

  const loadFile = useCallback(
    async (path: string) => {
      if (!projectRoot || !path) return;
      setLoadingFile(true);
      try {
        const resp = await fetch(
          `/api/agent-file?project_root=${encodeURIComponent(projectRoot)}&path=${encodeURIComponent(path)}`
        );
        if (!resp.ok) return;
        const data = await resp.json();
        setContent(data.content || '');
        setSavedContent(data.content || '');
        setValidationError(data.valid ? null : data.error || 'Invalid markdown frontmatter.');
      } finally {
        setLoadingFile(false);
      }
    },
    [projectRoot]
  );

  useEffect(() => { fetchList(); }, [fetchList]);

  useEffect(() => {
    if (selectedPath) loadFile(selectedPath);
  }, [selectedPath, loadFile]);

  const selectFile = (path: string) => {
    if (dirty && !confirm('Discard unsaved changes?')) return;
    setSelectedPath(path);
  };

  const saveFile = async () => {
    if (!projectRoot || !selectedPath) return;
    setSaving(true);
    setValidationError(null);
    try {
      const resp = await fetch('/api/agent-file', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: projectRoot, path: selectedPath, content }),
      });
      if (!resp.ok) {
        const text = await resp.text();
        setValidationError(text || 'Save failed.');
        return;
      }
      setSavedContent(content);
      setValidationError(null);
      await fetchList();
      onChanged?.();
    } finally {
      setSaving(false);
    }
  };

  const createFile = async () => {
    if (!projectRoot) return;
    const raw = prompt('New agent filename (example: reviewer.md):', 'new-agent.md');
    if (!raw) return;
    const filename = raw.trim().replace(/\\/g, '/');
    if (!filename) return;
    const name = filename.replace(/\.md$/i, '').split('/').pop() || 'new-agent';
    const path = filename.startsWith('agents/') ? filename : `agents/${filename}`;
    const template = defaultAgentTemplate(name.toLowerCase());
    const resp = await fetch('/api/agent-file', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ project_root: projectRoot, path, content: template }),
    });
    if (!resp.ok) {
      const text = await resp.text();
      setValidationError(text || 'Create failed.');
      return;
    }
    await fetchList();
    setSelectedPath(path);
    onChanged?.();
  };

  const deleteFile = async () => {
    if (!projectRoot || !selectedPath) return;
    if (!confirm(`Delete ${selectedPath}?`)) return;
    const resp = await fetch('/api/agent-file', {
      method: 'DELETE',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ project_root: projectRoot, path: selectedPath }),
    });
    if (!resp.ok) {
      const text = await resp.text();
      setValidationError(text || 'Delete failed.');
      return;
    }
    await fetchList();
    onChanged?.();
  };

  const togglePolicy = (key: string) => {
    const next = currentPolicies.includes(key)
      ? currentPolicies.filter((p) => p !== key)
      : [...currentPolicies, key];
    setContent(updatePolicyInContent(content, next));
  };

  const setDelegateTargetsValue = (val: string) => {
    setContent(updateDelegateTargetsInContent(content, val));
  };

  return (
    <div className="flex h-full min-h-0">
      {/* Sidebar: agent list */}
      <aside className="w-64 border-r border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-black/20 flex flex-col shrink-0">
        <div className="px-3 py-2 border-b border-slate-200 dark:border-white/10 flex items-center justify-between">
          <div className="text-xs font-bold uppercase tracking-wide text-slate-600 dark:text-slate-300">Agent Specs</div>
          <button className="p-1.5 rounded hover:bg-slate-200 dark:hover:bg-white/10" onClick={createFile} title="New agent file">
            <FilePlus2 size={14} />
          </button>
        </div>
        <div className="flex-1 overflow-y-auto p-2 space-y-1">
          {loadingList ? (
            <div className="text-xs text-slate-500 p-2">Loading...</div>
          ) : files.length === 0 ? (
            <div className="text-xs text-slate-500 p-2">No agent markdown files found.</div>
          ) : (
            files.map((file) => {
              return (
                <button
                  key={file.path}
                  onClick={() => selectFile(file.path)}
                  className={`w-full text-left px-2 py-1.5 rounded border ${
                    selectedPath === file.path
                      ? 'border-blue-300 bg-blue-50 dark:bg-blue-500/10 text-blue-700 dark:text-blue-300'
                      : 'border-transparent hover:border-slate-200 dark:hover:border-white/10'
                  }`}
                >
                  <div className="flex items-center gap-1.5">
                    <span className="text-xs font-semibold truncate flex-1">{file.agent_id}</span>
                  </div>
                  <div className="text-[11px] text-slate-500 truncate">{file.path}</div>
                </button>
              );
            })
          )}
        </div>
      </aside>

      {/* Editor area */}
      <section className="flex-1 min-w-0 overflow-y-auto">
        {/* Toolbar */}
        <div className="sticky top-0 z-10 px-3 py-2 border-b border-slate-200 dark:border-white/10 flex items-center justify-between bg-white dark:bg-[#0a0a0a]">
          <div className="flex items-center gap-2 min-w-0">
            <FileText size={14} className="text-slate-500 shrink-0" />
            <span className="text-xs font-mono truncate">{selectedPath || 'No file selected'}</span>
          </div>
          <div className="flex items-center gap-1.5">
            {dirty && <span className="text-[11px] text-amber-600">Unsaved</span>}
            <button
              onClick={saveFile}
              disabled={!selectedPath || saving || loadingFile}
              className="px-2 py-1 rounded text-xs border border-slate-200 dark:border-white/10 hover:bg-slate-50 dark:hover:bg-white/5 disabled:opacity-50"
              title="Save"
            >
              <span className="inline-flex items-center gap-1"><Save size={12} /> Save</span>
            </button>
            <button
              onClick={deleteFile}
              disabled={!selectedPath || saving}
              className="px-2 py-1 rounded text-xs border border-red-200 text-red-600 hover:bg-red-50 disabled:opacity-50"
              title="Delete"
            >
              <span className="inline-flex items-center gap-1"><Trash2 size={12} /> Delete</span>
            </button>
          </div>
        </div>

        {validationError && (
          <div className="px-3 py-2 text-xs bg-red-50 text-red-700 border-b border-red-100">{validationError}</div>
        )}

        {/* Policy editor */}
        {selectedPath && (
          <div className="px-3 py-2 border-b border-slate-200 dark:border-white/10 flex items-center gap-4 bg-slate-50/50 dark:bg-white/[0.02]">
            <span className="text-[10px] font-bold uppercase tracking-wider text-slate-500">Policy</span>
            {policyBadges.map(({ key, label, cls }) => (
              <label key={key} className="inline-flex items-center gap-1.5 cursor-pointer">
                <input
                  type="checkbox"
                  checked={currentPolicies.includes(key)}
                  onChange={() => togglePolicy(key)}
                  className="w-3 h-3 rounded border-slate-300 text-blue-600"
                />
                <span className={`text-[10px] font-bold px-1.5 py-0.5 rounded ${cls}`}>{label}</span>
                <span className="text-[10px] text-slate-500">{key}</span>
              </label>
            ))}
            {currentPolicies.includes('Delegate') && (
              <div className="flex items-center gap-1.5 ml-2">
                <span className="text-[10px] text-slate-500">Targets:</span>
                <input
                  className="bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-2 py-0.5 text-[10px] w-40 outline-none focus:ring-1 focus:ring-blue-500/50"
                  value={delegateTargets}
                  onChange={(e) => setDelegateTargetsValue(e.target.value)}
                  placeholder="coder, search"
                />
              </div>
            )}
          </div>
        )}

        {/* Live preview editor */}
        <div>
          {loadingFile ? (
            <div className="h-full flex items-center justify-center text-xs text-slate-500">Loading file...</div>
          ) : (
            <CM6Editor value={content} onChange={setContent} livePreview />
          )}
        </div>
      </section>
    </div>
  );
};
