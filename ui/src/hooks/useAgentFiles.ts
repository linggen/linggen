// The agent-spec editor's data: list agents/*.md, load one, save, create,
// delete. Shared by the Settings tab and the modal editor.
import { useCallback, useEffect, useMemo, useState } from 'react';
import type { AgentFileInfo } from '../types';
import { apiErrorMessage } from '../lib/api';
import { confirmDialog, promptDialog } from '../lib/confirmDialog';
import { agentFiles } from '../lib/endpoints';

const defaultAgentTemplate = (agentName: string) => `---
name: ${agentName}
description: ${agentName} agent.
tools: [Read]
model: inherit
---

You are linggen '${agentName}'.
`;

/** `enabled` false (a closed modal) skips loading. */
export function useAgentFiles(projectRoot: string, enabled: boolean, onChanged?: () => void) {
  const [files, setFiles] = useState<AgentFileInfo[]>([]);
  const [selectedPath, setSelectedPath] = useState('');
  const [content, setContent] = useState('');
  const [savedContent, setSavedContent] = useState('');
  const [loadingList, setLoadingList] = useState(false);
  const [loadingFile, setLoadingFile] = useState(false);
  const [saving, setSaving] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);
  const dirty = useMemo(() => content !== savedContent, [content, savedContent]);

  const fetchList = useCallback(async () => {
    if (!projectRoot) return;
    setLoadingList(true);
    try {
      const data = await agentFiles.list(projectRoot);
      setFiles(data);
      if (data.length === 0) {
        setSelectedPath('');
        setContent('');
        setSavedContent('');
        return;
      }
      setSelectedPath((prev) => (!prev || !data.some((item) => item.path === prev) ? data[0].path : prev));
    } catch { /* keep the last list */ } finally {
      setLoadingList(false);
    }
  }, [projectRoot]);

  const loadFile = useCallback(async (path: string) => {
    if (!projectRoot || !path) return;
    setLoadingFile(true);
    try {
      const data = await agentFiles.read(projectRoot, path);
      setContent(data.content || '');
      setSavedContent(data.content || '');
      setValidationError(data.valid ? null : data.error || 'Invalid markdown frontmatter.');
    } catch { /* leave the editor as it was */ } finally {
      setLoadingFile(false);
    }
  }, [projectRoot]);

  useEffect(() => { if (enabled) fetchList(); }, [enabled, fetchList]);
  useEffect(() => { if (enabled && selectedPath) loadFile(selectedPath); }, [enabled, selectedPath, loadFile]);

  const selectFile = async (path: string) => {
    if (dirty && !(await confirmDialog('Discard unsaved changes?'))) return;
    setSelectedPath(path);
  };

  /** Run a write; on failure show the server's text, on success refresh. */
  const write = async (call: () => Promise<unknown>, failure: string) => {
    try {
      await call();
    } catch (e) {
      setValidationError(apiErrorMessage(e) || failure);
      return false;
    }
    await fetchList();
    onChanged?.();
    return true;
  };

  const saveFile = async () => {
    if (!projectRoot || !selectedPath) return;
    setSaving(true);
    setValidationError(null);
    try {
      if (await write(() => agentFiles.save(projectRoot, selectedPath, content), 'Save failed.')) {
        setSavedContent(content);
      }
    } finally {
      setSaving(false);
    }
  };

  const createFile = async () => {
    if (!projectRoot) return;
    // An in-page prompt: window.prompt() returns null instantly in the
    // Tauri shell, so New did nothing inside Linggen.app.
    const raw = await promptDialog('New agent filename (example: reviewer.md):', 'new-agent.md');
    const filename = raw?.trim().replace(/\\/g, '/');
    if (!filename) return;
    const name = filename.replace(/\.md$/i, '').split('/').pop() || 'new-agent';
    const path = filename.startsWith('agents/') ? filename : `agents/${filename}`;
    if (await write(() => agentFiles.save(projectRoot, path, defaultAgentTemplate(name.toLowerCase())), 'Create failed.')) {
      setSelectedPath(path);
    }
  };

  const deleteFile = async () => {
    if (!projectRoot || !selectedPath) return;
    if (!(await confirmDialog(`Delete ${selectedPath}?`))) return;
    await write(() => agentFiles.remove(projectRoot, selectedPath), 'Delete failed.');
  };

  return {
    files, selectedPath, content, setContent, loadingList, loadingFile, saving, validationError, dirty,
    selectFile, saveFile, createFile, deleteFile,
  };
}
