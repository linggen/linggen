import React, { useCallback, useEffect, useState } from 'react';
import {
  ArrowLeft,
  ChevronDown,
  ChevronRight,
  File,
  Folder,
  FolderOpen,
  Save,
  Trash2,
} from 'lucide-react';
import { CM6Editor } from './CM6Editor';
import type { StorageEntry, StorageRoot } from '../types';
import { cn } from '../lib/cn';

// ---------------------------------------------------------------------------
// API helpers
// ---------------------------------------------------------------------------

async function fetchRoots(): Promise<StorageRoot[]> {
  const res = await fetch('/api/storage/roots');
  if (!res.ok) return [];
  return res.json();
}

async function fetchTree(root: string, path: string): Promise<StorageEntry[]> {
  const params = new URLSearchParams({ root });
  if (path) params.set('path', path);
  const res = await fetch(`/api/storage/tree?${params}`);
  if (!res.ok) return [];
  const data = await res.json();
  return data.entries ?? [];
}

async function fetchFile(root: string, path: string): Promise<{ content: string; size: number; modified: number } | null> {
  const params = new URLSearchParams({ root, path });
  const res = await fetch(`/api/storage/file?${params}`);
  if (!res.ok) return null;
  return res.json();
}

async function saveFile(root: string, path: string, content: string): Promise<boolean> {
  const res = await fetch('/api/storage/file', {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ root, path, content }),
  });
  return res.ok;
}

async function deleteFile(root: string, path: string): Promise<boolean> {
  const params = new URLSearchParams({ root, path });
  const res = await fetch(`/api/storage/file?${params}`, { method: 'DELETE' });
  return res.ok;
}

// ---------------------------------------------------------------------------
// FileTree
// ---------------------------------------------------------------------------

const FileTreeNode: React.FC<{
  entry: StorageEntry;
  root: string;
  depth: number;
  expanded: Set<string>;
  childrenCache: Map<string, StorageEntry[]>;
  selected: string | null;
  onToggleDir: (path: string) => void;
  onSelectFile: (path: string) => void;
}> = ({ entry, root, depth, expanded, childrenCache, selected, onToggleDir, onSelectFile }) => {
  const isExpanded = expanded.has(entry.path);
  const children = childrenCache.get(entry.path);

  if (entry.is_dir) {
    return (
      <>
        <button
          onClick={() => onToggleDir(entry.path)}
          className={cn(
            'flex items-center gap-1.5 w-full px-2 py-1 text-left text-xs hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors',
          )}
          style={{ paddingLeft: `${depth * 12 + 8}px` }}
        >
          {isExpanded ? <ChevronDown size={12} className="shrink-0 text-slate-400" /> : <ChevronRight size={12} className="shrink-0 text-slate-400" />}
          {isExpanded ? <FolderOpen size={13} className="shrink-0 text-amber-500" /> : <Folder size={13} className="shrink-0 text-amber-500" />}
          <span className="truncate">{entry.name}</span>
        </button>
        {isExpanded && children && children.map((child) => (
          <FileTreeNode
            key={child.path}
            entry={child}
            root={root}
            depth={depth + 1}
            expanded={expanded}
            childrenCache={childrenCache}
            selected={selected}
            onToggleDir={onToggleDir}
            onSelectFile={onSelectFile}
          />
        ))}
      </>
    );
  }

  return (
    <button
      onClick={() => onSelectFile(entry.path)}
      className={cn(
        'flex items-center gap-1.5 w-full px-2 py-1 text-left text-xs rounded transition-colors',
        selected === entry.path
          ? 'bg-blue-500/10 text-blue-600 dark:text-blue-400'
          : 'hover:bg-slate-100 dark:hover:bg-white/5',
      )}
      style={{ paddingLeft: `${depth * 12 + 8 + 14}px` }}
    >
      <File size={13} className="shrink-0 text-slate-400" />
      <span className="truncate">{entry.name}</span>
    </button>
  );
};

// ---------------------------------------------------------------------------
// StoragePage
// ---------------------------------------------------------------------------

export const StoragePage: React.FC<{
  onBack: () => void;
  embedded?: boolean;
}> = ({ onBack, embedded }) => {
  const [roots, setRoots] = useState<StorageRoot[]>([]);
  const [activeRoot, setActiveRoot] = useState('');
  const [childrenCache, setChildrenCache] = useState<Map<string, StorageEntry[]>>(new Map());
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [fileContent, setFileContent] = useState('');
  const [savedContent, setSavedContent] = useState('');
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);

  const isDirty = fileContent !== savedContent;
  const isMarkdown = selectedFile?.endsWith('.md') ?? false;

  // --- Load roots on mount ---
  useEffect(() => {
    fetchRoots().then((r) => {
      setRoots(r);
      if (r.length > 0) setActiveRoot(r[0].path);
    });
  }, []);

  // --- Load top-level tree when root changes ---
  useEffect(() => {
    if (!activeRoot) return;
    setChildrenCache(new Map());
    setExpandedDirs(new Set());
    setSelectedFile(null);
    setFileContent('');
    setSavedContent('');

    fetchTree(activeRoot, '').then((entries) => {
      setChildrenCache(new Map([['', entries]]));
    });
  }, [activeRoot]);

  // --- Dir toggle ---
  const toggleDir = useCallback(
    async (path: string) => {
      setExpandedDirs((prev) => {
        const next = new Set(prev);
        if (next.has(path)) {
          next.delete(path);
        } else {
          next.add(path);
        }
        return next;
      });
      // Lazy-fetch if not cached
      if (!childrenCache.has(path)) {
        const entries = await fetchTree(activeRoot, path);
        setChildrenCache((prev) => new Map(prev).set(path, entries));
      }
    },
    [activeRoot, childrenCache],
  );

  // --- File select ---
  const selectFile = useCallback(
    async (path: string) => {
      if (path === selectedFile) return;
      setLoading(true);
      setSelectedFile(path);
      const data = await fetchFile(activeRoot, path);
      if (data) {
        setFileContent(data.content);
        setSavedContent(data.content);
      } else {
        setFileContent('');
        setSavedContent('');
      }
      setLoading(false);
    },
    [activeRoot, selectedFile],
  );

  // --- Save ---
  const handleSave = useCallback(async () => {
    if (!selectedFile || !isDirty) return;
    setSaving(true);
    const ok = await saveFile(activeRoot, selectedFile, fileContent);
    if (ok) setSavedContent(fileContent);
    setSaving(false);
  }, [activeRoot, selectedFile, fileContent, isDirty]);

  // --- Delete ---
  const handleDelete = useCallback(async () => {
    if (!selectedFile) return;
    if (!confirm(`Delete ${selectedFile}?`)) return;
    const ok = await deleteFile(activeRoot, selectedFile);
    if (ok) {
      setSelectedFile(null);
      setFileContent('');
      setSavedContent('');
      // Refresh parent dir
      const parentPath = selectedFile.includes('/')
        ? selectedFile.substring(0, selectedFile.lastIndexOf('/'))
        : '';
      const entries = await fetchTree(activeRoot, parentPath);
      setChildrenCache((prev) => new Map(prev).set(parentPath, entries));
    }
  }, [activeRoot, selectedFile]);

  // Top-level entries for the tree
  const topEntries = childrenCache.get('') ?? [];

  const fileActions = selectedFile && (
    <div className="flex items-center gap-2">
      <button
        onClick={handleSave}
        disabled={!isDirty || saving}
        className={cn(
          'flex items-center gap-1.5 px-3 py-1 rounded text-xs font-medium transition-colors',
          isDirty
            ? 'bg-blue-600 text-white hover:bg-blue-700'
            : 'bg-slate-200 dark:bg-white/5 text-slate-400 cursor-not-allowed',
        )}
      >
        <Save size={12} />
        {saving ? 'Saving...' : 'Save'}
      </button>
      <button
        onClick={handleDelete}
        className="flex items-center gap-1.5 px-3 py-1 rounded text-xs font-medium text-red-500 hover:bg-red-500/10 transition-colors"
      >
        <Trash2 size={12} />
        Delete
      </button>
    </div>
  );

  if (embedded) {
    return (
      <div className="flex flex-col h-full text-slate-900 dark:text-slate-200">
        {/* Toolbar for embedded mode */}
        <div className="flex items-center justify-end px-4 py-2 border-b border-slate-200 dark:border-white/5 shrink-0">
          {fileActions}
        </div>
        <div className="flex flex-1 overflow-hidden">
          {/* Sidebar */}
          <aside className="w-60 border-r border-slate-200 dark:border-white/5 flex flex-col bg-white dark:bg-[#0f0f0f] shrink-0 overflow-hidden">
            <div className="px-3 py-2 border-b border-slate-200 dark:border-white/5">
              <select
                value={activeRoot}
                onChange={(e) => setActiveRoot(e.target.value)}
                className="w-full text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-2 py-1.5 outline-none focus:ring-1 focus:ring-blue-500"
              >
                {roots.map((r) => (
                  <option key={r.path} value={r.path}>
                    {r.label} — {r.path.replace(/^\/Users\/[^/]+/, '~')}
                  </option>
                ))}
              </select>
            </div>
            <div className="flex-1 overflow-y-auto py-1">
              {topEntries.map((entry) => (
                <FileTreeNode
                  key={entry.path}
                  entry={entry}
                  root={activeRoot}
                  depth={0}
                  expanded={expandedDirs}
                  childrenCache={childrenCache}
                  selected={selectedFile}
                  onToggleDir={toggleDir}
                  onSelectFile={selectFile}
                />
              ))}
              {topEntries.length === 0 && (
                <div className="text-xs text-slate-400 text-center py-6">Empty</div>
              )}
            </div>
          </aside>
          {/* Editor panel */}
          <div className="flex-1 flex flex-col overflow-hidden">
            {selectedFile ? (
              <>
                <div className="px-4 py-1.5 text-[12px] text-slate-400 border-b border-slate-200 dark:border-white/5 bg-white dark:bg-[#0f0f0f] truncate shrink-0">
                  {selectedFile}
                  {isDirty && <span className="ml-2 text-amber-500 font-medium">modified</span>}
                </div>
                <div className="flex-1 overflow-y-auto">
                  {loading ? (
                    <div className="flex items-center justify-center h-full text-sm text-slate-400">Loading...</div>
                  ) : (
                    <CM6Editor value={fileContent} onChange={setFileContent} livePreview={isMarkdown} filePath={selectedFile ?? undefined} />
                  )}
                </div>
              </>
            ) : (
              <div className="flex items-center justify-center h-full text-sm text-slate-400">Select a file to view</div>
            )}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-screen bg-slate-50 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200">
      {/* Header */}
      <header className="flex items-center justify-between px-4 py-2.5 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md shrink-0">
        <div className="flex items-center gap-3">
          <button onClick={onBack} className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors text-slate-500">
            <ArrowLeft size={16} />
          </button>
          <h1 className="text-sm font-bold tracking-tight">Storage</h1>
        </div>
        {fileActions}
      </header>

      {/* Body */}
      <div className="flex flex-1 overflow-hidden">
        {/* Sidebar */}
        <aside className="w-60 border-r border-slate-200 dark:border-white/5 flex flex-col bg-white dark:bg-[#0f0f0f] shrink-0 overflow-hidden">
          {/* Root selector */}
          <div className="px-3 py-2 border-b border-slate-200 dark:border-white/5">
            <select
              value={activeRoot}
              onChange={(e) => setActiveRoot(e.target.value)}
              className="w-full text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-2 py-1.5 outline-none focus:ring-1 focus:ring-blue-500"
            >
              {roots.map((r) => (
                <option key={r.path} value={r.path}>
                  {r.label} — {r.path.replace(/^\/Users\/[^/]+/, '~')}
                </option>
              ))}
            </select>
          </div>

          {/* File tree */}
          <div className="flex-1 overflow-y-auto py-1">
            {topEntries.map((entry) => (
              <FileTreeNode
                key={entry.path}
                entry={entry}
                root={activeRoot}
                depth={0}
                expanded={expandedDirs}
                childrenCache={childrenCache}
                selected={selectedFile}
                onToggleDir={toggleDir}
                onSelectFile={selectFile}
              />
            ))}
            {topEntries.length === 0 && (
              <div className="text-xs text-slate-400 text-center py-6">Empty</div>
            )}
          </div>
        </aside>

        {/* Editor panel */}
        <div className="flex-1 flex flex-col overflow-hidden">
          {selectedFile ? (
            <>
              {/* Breadcrumb */}
              <div className="px-4 py-1.5 text-[12px] text-slate-400 border-b border-slate-200 dark:border-white/5 bg-white dark:bg-[#0f0f0f] truncate shrink-0">
                {selectedFile}
                {isDirty && <span className="ml-2 text-amber-500 font-medium">modified</span>}
              </div>
              <div className="flex-1 overflow-y-auto">
                {loading ? (
                  <div className="flex items-center justify-center h-full text-sm text-slate-400">
                    Loading...
                  </div>
                ) : (
                  <CM6Editor
                    value={fileContent}
                    onChange={setFileContent}
                    livePreview={isMarkdown}
                    filePath={selectedFile ?? undefined}
                  />
                )}
              </div>
            </>
          ) : (
            <div className="flex items-center justify-center h-full text-sm text-slate-400">
              Select a file to view
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
