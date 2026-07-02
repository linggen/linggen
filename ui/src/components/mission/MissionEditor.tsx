import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { FileText, Save, Trash2 } from 'lucide-react';
import type { CronMission } from '../../types';
import { CM6Editor } from '../CM6Editor';
import { deleteMission, getMissionFile, saveMissionFile } from '../../lib/missions-api';
import { confirmDialog, promptDialog } from '../../lib/confirmDialog';

/** Default body for a brand-new mission. Frontmatter is the source of truth
 *  for all metadata; users edit it directly. Keep this minimal — the user
 *  is expected to fill in schedule, prompt, and any permission/allowed-tools
 *  themselves. */
const defaultMissionTemplate = (id: string) => `---
name: ${id}
description: One-line summary shown in the mission list.
schedule: "0 3 * * *"
enabled: true
allowed-tools:
  - Bash
---

# ${id}

Step-by-step instructions for the mission go here.
`;

export const MissionEditor: React.FC<{
  editing: CronMission | null;
  workingFolders?: string[];
  onSave: (mission: CronMission) => void;
  onCancel: () => void;
  onViewAgent?: () => void;
}> = ({ editing, onSave, onCancel }) => {
  // For a brand-new mission, ask the user for an id up-front and seed with a
  // template. We intentionally drive everything off `missionId` + raw content
  // — no parallel structured state.
  const [missionId, setMissionId] = useState<string>(editing?.id || '');
  const [content, setContent] = useState<string>('');
  const [savedContent, setSavedContent] = useState<string>('');
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [needsIdPrompt, setNeedsIdPrompt] = useState(!editing);

  const dirty = useMemo(() => content !== savedContent, [content, savedContent]);

  const loadFile = useCallback(async (id: string) => {
    setLoading(true);
    setError(null);
    try {
      const data = await getMissionFile(id);
      setContent(data.content);
      setSavedContent(data.content);
    } catch (e: any) {
      setError(e?.message || 'Failed to load mission file.');
    } finally {
      setLoading(false);
    }
  }, []);

  // Load existing mission's raw md on first mount.
  useEffect(() => {
    if (editing?.id) loadFile(editing.id);
  }, [editing?.id, loadFile]);

  // New-mission flow: prompt for an id, seed template.
  const startNewMission = useCallback(async () => {
    const raw = await promptDialog(
      'New mission id (lowercase, no spaces — used as the folder name):',
      'new-mission',
    );
    if (!raw) {
      onCancel();
      return;
    }
    const id = raw.trim().toLowerCase().replace(/[^a-z0-9_-]/g, '-');
    if (!id) {
      onCancel();
      return;
    }
    setMissionId(id);
    const template = defaultMissionTemplate(id);
    setContent(template);
    setSavedContent('');
    setNeedsIdPrompt(false);
  }, [onCancel]);

  useEffect(() => {
    if (needsIdPrompt) startNewMission();
  }, [needsIdPrompt, startNewMission]);

  const handleSave = async () => {
    if (!missionId) return;
    setSaving(true);
    setError(null);
    try {
      const mission = await saveMissionFile(missionId, content);
      setSavedContent(content);
      onSave(mission);
    } catch (e: any) {
      setError(e?.message || 'Failed to save mission.');
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    if (!editing?.id) return;
    if (!(await confirmDialog(`Delete mission "${editing.id}"? This removes the folder under ~/.linggen/missions/.`))) return;
    try {
      await deleteMission(editing.id);
      onCancel();
    } catch (e: any) {
      setError(e?.message || 'Failed to delete mission.');
    }
  };

  const handleCancel = async () => {
    if (dirty && !(await confirmDialog('Discard unsaved changes?'))) return;
    onCancel();
  };

  if (needsIdPrompt) {
    return <div className="p-6 text-sm text-slate-500">Naming new mission...</div>;
  }

  const filePath = `~/.linggen/missions/${missionId}/mission.md`;

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="sticky top-0 z-10 px-4 py-2 border-b border-slate-200 dark:border-white/10 flex items-center justify-between bg-white dark:bg-[#0a0a0a]">
        <div className="flex items-center gap-2 min-w-0">
          <FileText size={14} className="text-slate-500 shrink-0" />
          <span className="text-xs font-mono truncate">{filePath}</span>
          {dirty && <span className="text-[12px] text-amber-600 ml-2">Unsaved</span>}
        </div>
        <div className="flex items-center gap-1.5">
          <button
            onClick={handleSave}
            disabled={saving || loading || !content.trim()}
            className="px-3 py-1.5 rounded text-xs font-semibold border border-blue-500/40 bg-blue-500/10 text-blue-700 dark:text-blue-300 hover:bg-blue-500/20 disabled:opacity-50"
          >
            <span className="inline-flex items-center gap-1"><Save size={12} /> {saving ? 'Saving…' : editing ? 'Save' : 'Create'}</span>
          </button>
          {editing && (
            <button
              onClick={handleDelete}
              className="px-2 py-1.5 rounded text-xs border border-red-200 text-red-600 hover:bg-red-50 dark:hover:bg-red-500/10"
            >
              <span className="inline-flex items-center gap-1"><Trash2 size={12} /> Delete</span>
            </button>
          )}
          <button
            onClick={handleCancel}
            className="px-3 py-1.5 rounded text-xs border border-slate-200 dark:border-white/10 text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5"
          >
            Cancel
          </button>
        </div>
      </div>

      {error && (
        <div className="px-4 py-2 text-xs bg-red-50 dark:bg-red-500/10 text-red-700 dark:text-red-300 border-b border-red-100 dark:border-red-500/20">
          {error}
        </div>
      )}

      <div className="flex-1 min-h-0 overflow-y-auto">
        {loading ? (
          <div className="p-6 text-xs text-slate-500">Loading mission.md…</div>
        ) : (
          <CM6Editor value={content} onChange={setContent} livePreview />
        )}
      </div>
    </div>
  );
};
