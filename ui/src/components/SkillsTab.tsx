import React, { useCallback, useEffect, useRef, useState } from 'react';
import { AlertTriangle, ArrowUpRight, Book, Check, ChevronRight, Download, ExternalLink, FilePlus2, Package, Pencil, RefreshCw, Save, Search, Shield, ShieldAlert, Sparkles, Trash2, Wrench, X, Zap } from 'lucide-react';
import type { BuiltInSkillInfo, MarketplaceSkill, SkillInfoFull, SkillFileInfo } from '../types';
import { CM6Editor } from './CM6Editor';

/* ── Helpers ── */
function formatRelativeDate(dateStr: string): string {
  try {
    const date = new Date(dateStr);
    const now = Date.now();
    const diffMs = now - date.getTime();
    if (diffMs < 0 || isNaN(diffMs)) return dateStr;
    const mins = Math.floor(diffMs / 60_000);
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    if (days < 30) return `${days}d ago`;
    const months = Math.floor(days / 30);
    if (months < 12) return `${months}mo ago`;
    return `${Math.floor(months / 12)}y ago`;
  } catch {
    return dateStr;
  }
}

/* ── Source badge colors by type ── */
const sourceBadgeCls: Record<string, string> = {
  Global: 'bg-indigo-500/10 text-indigo-600 dark:text-indigo-400 border-indigo-200/50 dark:border-indigo-500/20',
  Project: 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border-emerald-200/50 dark:border-emerald-500/20',
  Compat: 'bg-slate-500/8 text-slate-500 dark:text-slate-400 border-slate-200/50 dark:border-slate-500/20',
};

/* ── Left-border accent colors for source groups ── */
const sourceAccentCls: Record<string, string> = {
  Global: 'border-l-indigo-400 dark:border-l-indigo-500',
  Project: 'border-l-emerald-400 dark:border-l-emerald-500',
  Compat: 'border-l-slate-300 dark:border-l-slate-500',
};

/** Get a short subfolder badge label for a skill based on its source. */
function skillSubfolderLabel(skill: SkillInfoFull): string {
  const t = skill.source?.type || 'Global';
  if (t === 'Compat') {
    return (skill.source as { type: string; label?: string })?.label || 'Compat';
  }
  if (t === 'Global') return 'Linggen';
  // Project skills — we don't know the exact subfolder from the source alone,
  // so just show "Project"
  return 'Project';
}

function isGlobalOrCompat(skill: SkillInfoFull): boolean {
  const t = skill.source?.type || 'Global';
  return t === 'Global' || t === 'Compat';
}

const defaultSkillTemplate = (name: string) => `---
name: ${name}
description: ${name} skill.
tools: []
---

Skill instructions go here.
`;

export const SkillsTab: React.FC<{
  projectRoot: string;
}> = ({ projectRoot }) => {
  const [allSkills, setAllSkills] = useState<SkillInfoFull[]>([]);
  const [skillFiles, setSkillFiles] = useState<SkillFileInfo[]>([]);
  const [expandedSkills, setExpandedSkills] = useState<Set<string>>(new Set());
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
  const [editingSkill, setEditingSkill] = useState<string | null>(null);
  const [editContent, setEditContent] = useState<string>('');
  const [savedEditContent, setSavedEditContent] = useState<string>('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  // Marketplace state
  const [mpQuery, setMpQuery] = useState('');
  const [mpResults, setMpResults] = useState<MarketplaceSkill[]>([]);
  const [mpLoading, setMpLoading] = useState(false);
  const [mpInstalling, setMpInstalling] = useState<Set<string>>(new Set());
  const [mpUninstalling, setMpUninstalling] = useState<Set<string>>(new Set());
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Trending community skills (auto-loaded)
  const [trendingSkills, setTrendingSkills] = useState<MarketplaceSkill[]>([]);
  const fetchTrending = useCallback(async () => {
    try {
      const resp = await fetch('/api/community-skills/search?q=agent');
      if (resp.ok) {
        const skills: MarketplaceSkill[] = await resp.json();
        // Show top 10 by install count, preferring ones with descriptions
        const sorted = skills
          .sort((a, b) => (b.install_count || 0) - (a.install_count || 0))
          .slice(0, 10);
        setTrendingSkills(sorted);
      }
    } catch { /* ignore */ }
  }, []);

  // Built-in skills state
  const [builtInSkills, setBuiltInSkills] = useState<BuiltInSkillInfo[]>([]);
  const [biInstalling, setBiInstalling] = useState<Set<string>>(new Set());
  const fetchBuiltInSkills = useCallback(async (refresh = false) => {
    try {
      const url = refresh ? '/api/builtin-skills?refresh=true' : '/api/builtin-skills';
      const resp = await fetch(url);
      if (resp.ok) setBuiltInSkills(await resp.json());
    } catch { /* ignore */ }
  }, []);

  const installBuiltInSkill = async (name: string) => {
    setBiInstalling((prev) => new Set(prev).add(name));
    setError(null);
    try {
      const resp = await fetch('/api/builtin-skills/install', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name }),
      });
      if (!resp.ok) {
        setError(await resp.text());
      } else {
        await Promise.all([fetchSkills(), fetchSkillFiles(), fetchBuiltInSkills()]);
      }
    } catch (e) {
      setError(String(e));
    }
    setBiInstalling((prev) => {
      const next = new Set(prev);
      next.delete(name);
      return next;
    });
  };

  const fetchSkills = useCallback(async () => {
    try {
      const resp = await fetch('/api/skills');
      if (resp.ok) setAllSkills(await resp.json());
    } catch { /* ignore */ }
  }, []);

  const fetchSkillFiles = useCallback(async () => {
    if (!projectRoot) return;
    try {
      const resp = await fetch(`/api/skill-files?project_root=${encodeURIComponent(projectRoot)}`);
      if (resp.ok) setSkillFiles(await resp.json());
    } catch { /* ignore */ }
  }, [projectRoot]);

  useEffect(() => {
    fetchSkills();
    fetchSkillFiles();
    fetchBuiltInSkills();
    fetchTrending();
  }, [fetchSkills, fetchSkillFiles, fetchBuiltInSkills, fetchTrending]);

  const searchCommunity = async (q: string) => {
    if (!q.trim()) {
      setMpResults([]);
      return;
    }
    setMpLoading(true);
    try {
      const resp = await fetch(`/api/community-skills/search?q=${encodeURIComponent(q)}`);
      if (resp.ok) setMpResults(await resp.json());
    } catch { /* ignore */ }
    setMpLoading(false);
  };

  const handleMpQueryChange = (val: string) => {
    setMpQuery(val);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => searchCommunity(val), 400);
  };

  const installedNames = new Set(allSkills.map((s) => s.name));

  const installMarketplaceSkill = async (skill: MarketplaceSkill, scope: 'project' | 'global' = 'global') => {
    setMpInstalling((prev) => new Set(prev).add(skill.name));
    setError(null);
    try {
      const resp = await fetch('/api/marketplace/install', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: skill.name,
          repo_url: skill.url || undefined,
          git_ref: skill.git_ref || undefined,
          scope,
          project_root: scope === 'project' ? projectRoot : undefined,
          force: false,
          source: skill.source_registry || undefined,
        }),
      });
      if (!resp.ok) {
        setError(await resp.text());
      } else {
        await Promise.all([fetchSkills(), fetchSkillFiles()]);
      }
    } catch (e) {
      setError(String(e));
    }
    setMpInstalling((prev) => {
      const next = new Set(prev);
      next.delete(skill.name);
      return next;
    });
  };

  const uninstallMarketplaceSkill = async (name: string, scope: 'project' | 'global' = 'global') => {
    setMpUninstalling((prev) => new Set(prev).add(name));
    setError(null);
    try {
      const resp = await fetch('/api/marketplace/uninstall', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name,
          scope,
          project_root: scope === 'project' ? projectRoot : undefined,
        }),
      });
      if (!resp.ok) {
        setError(await resp.text());
      } else {
        await Promise.all([fetchSkills(), fetchSkillFiles()]);
      }
    } catch (e) {
      setError(String(e));
    }
    setMpUninstalling((prev) => {
      const next = new Set(prev);
      next.delete(name);
      return next;
    });
  };

  const [mpMoving, setMpMoving] = useState<Set<string>>(new Set());

  const handleMoveToGlobal = async (skillName: string) => {
    setMpMoving((prev) => new Set(prev).add(skillName));
    setError(null);
    try {
      const resp = await fetch('/api/marketplace/move-to-global', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: skillName, project_root: projectRoot }),
      });
      if (!resp.ok) {
        setError(await resp.text());
      } else {
        await Promise.all([fetchSkills(), fetchSkillFiles()]);
      }
    } catch (e) {
      setError(String(e));
    }
    setMpMoving((prev) => {
      const next = new Set(prev);
      next.delete(skillName);
      return next;
    });
  };

  const builtInNames = new Set(builtInSkills.map((bi) => bi.name));

  // Split into Global (includes Compat) and Project sections
  const globalSkills = allSkills.filter(isGlobalOrCompat);
  const projectSkills = allSkills.filter((s) => !isGlobalOrCompat(s));

  // Library: built-in skills filtered by search query, shown on top
  const filteredBuiltIn = mpQuery.trim()
    ? builtInSkills.filter((bi) =>
        bi.name.toLowerCase().includes(mpQuery.toLowerCase()) ||
        bi.description.toLowerCase().includes(mpQuery.toLowerCase())
      )
    : builtInSkills;

  // Library: marketplace results excluding built-in names (avoid duplication)
  // When no search query, show trending community skills
  const communitySource = mpQuery.trim() ? mpResults : trendingSkills;
  const filteredMpResults = communitySource.filter((s) => !builtInNames.has(s.name));

  const toggleExpanded = (name: string) => {
    setExpandedSkills((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  const toggleGroup = (source: string) => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(source)) next.delete(source);
      else next.add(source);
      return next;
    });
  };

  const startEditing = (skill: SkillInfoFull) => {
    const file = skillFiles.find((f) => f.name === skill.name);
    if (!file) return;
    loadSkillFile(file.path);
  };

  const loadSkillFile = async (path: string) => {
    try {
      const resp = await fetch(
        `/api/skill-file?project_root=${encodeURIComponent(projectRoot)}&path=${encodeURIComponent(path)}`
      );
      if (!resp.ok) return;
      const data = await resp.json();
      setEditContent(data.content || '');
      setSavedEditContent(data.content || '');
      setEditingSkill(path);
      setError(null);
    } catch { /* ignore */ }
  };

  const saveSkillFile = async () => {
    if (!editingSkill) return;
    setSaving(true);
    setError(null);
    try {
      const resp = await fetch('/api/skill-file', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: projectRoot, path: editingSkill, content: editContent }),
      });
      if (!resp.ok) {
        setError(await resp.text());
        return;
      }
      setSavedEditContent(editContent);
      await Promise.all([fetchSkills(), fetchSkillFiles()]);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const createSkillFile = async () => {
    const raw = prompt('New skill filename (example: my-skill.md):', 'new-skill.md');
    if (!raw) return;
    const filename = raw.trim();
    if (!filename) return;
    const name = filename.replace(/\.md$/i, '');
    const template = defaultSkillTemplate(name);
    try {
      const resp = await fetch('/api/skill-file', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: projectRoot, path: filename, content: template }),
      });
      if (!resp.ok) {
        setError(await resp.text());
        return;
      }
      await Promise.all([fetchSkills(), fetchSkillFiles()]);
      const data = await resp.json();
      if (data.path) loadSkillFile(data.path);
    } catch (e) {
      setError(String(e));
    }
  };

  const _deleteSkillFile = async (path: string) => {
    if (!confirm(`Delete skill file ${path}?`)) return;
    try {
      const resp = await fetch('/api/skill-file', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: projectRoot, path }),
      });
      if (!resp.ok) {
        setError(await resp.text());
        return;
      }
      if (editingSkill === path) {
        setEditingSkill(null);
        setEditContent('');
        setSavedEditContent('');
      }
      await Promise.all([fetchSkills(), fetchSkillFiles()]);
    } catch (e) {
      setError(String(e));
    }
  };

  const editDirty = editContent !== savedEditContent;

  const refreshAll = async () => {
    setRefreshing(true);
    const tasks: Promise<unknown>[] = [fetchSkills(), fetchSkillFiles(), fetchBuiltInSkills(true), fetchTrending()];
    if (mpQuery.trim()) tasks.push(searchCommunity(mpQuery));
    await Promise.all(tasks);
    setRefreshing(false);
  };

  // Editor mode
  if (editingSkill) {
    return (
      <div className="flex flex-col h-full min-h-0">
        <div className="px-4 py-2 border-b border-slate-200 dark:border-white/10 flex items-center justify-between bg-slate-50/50 dark:bg-white/[0.02]">
          <div className="flex items-center gap-2">
            <button
              onClick={() => {
                if (editDirty && !confirm('Discard unsaved changes?')) return;
                setEditingSkill(null);
              }}
              className="p-1 rounded hover:bg-slate-200 dark:hover:bg-white/10"
            >
              <X size={14} />
            </button>
            <span className="text-xs font-mono text-slate-600 dark:text-slate-300">{editingSkill}</span>
            {editDirty && <span className="text-[12px] text-amber-600">Unsaved</span>}
          </div>
          <div className="flex items-center gap-1.5">
            {error && <span className="text-[11px] text-red-500 max-w-60 truncate">{error}</span>}
            <button
              onClick={saveSkillFile}
              disabled={saving || !editDirty}
              className="px-2 py-1 rounded text-xs border border-slate-200 dark:border-white/10 hover:bg-slate-50 dark:hover:bg-white/5 disabled:opacity-50"
            >
              <span className="inline-flex items-center gap-1"><Save size={12} /> Save</span>
            </button>
          </div>
        </div>
        <div className="flex-1 min-h-0">
          <CM6Editor value={editContent} onChange={setEditContent} />
        </div>
      </div>
    );
  }

  const totalInstalled = allSkills.length;

  // ---------------------------------------------------------------------------
  // Two-column layout: Installed (left) | Marketplace (right)
  // ---------------------------------------------------------------------------
  return (
    <div className="flex flex-col h-full min-h-0">
      {/* ── Header bar ── */}
      <div className="flex items-center justify-between pb-4 shrink-0">
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2">
            <div className="w-7 h-7 rounded-lg bg-gradient-to-br from-indigo-500 to-blue-600 flex items-center justify-center shadow-sm shadow-indigo-500/20">
              <Zap size={14} className="text-white" />
            </div>
            <div>
              <h3 className="text-xs font-bold text-slate-800 dark:text-slate-100 leading-none">Skills</h3>
              <p className="text-[11px] text-slate-400 mt-0.5">{totalInstalled} installed</p>
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={refreshAll}
            disabled={refreshing}
            className="p-1.5 rounded-lg text-slate-400 hover:text-slate-600 dark:hover:text-slate-300 hover:bg-white dark:hover:bg-white/5 border border-transparent hover:border-slate-200 dark:hover:border-white/10 transition-all disabled:opacity-50"
            title="Refresh all"
          >
            <RefreshCw size={13} className={refreshing ? 'animate-spin' : ''} />
          </button>
          <button
            onClick={createSkillFile}
            className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-lg bg-white dark:bg-white/5 border border-slate-200 dark:border-white/10 text-slate-700 dark:text-slate-200 hover:border-blue-300 dark:hover:border-blue-500/30 hover:text-blue-600 dark:hover:text-blue-400 shadow-sm transition-all"
          >
            <FilePlus2 size={12} /> New Skill
          </button>
        </div>
      </div>

      {/* ── Error banner ── */}
      {error && (
        <div className="text-xs text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-500/10 border border-red-200/50 dark:border-red-500/20 rounded-lg px-3 py-2 mb-3 flex items-center justify-between shrink-0">
          <span className="truncate">{error}</span>
          <button onClick={() => setError(null)} className="ml-2 text-red-400 hover:text-red-600 shrink-0"><X size={12} /></button>
        </div>
      )}

      {/* ── Two-column body ── */}
      <div className="flex gap-4 flex-1 min-h-0">

        {/* ═══════ LEFT PANEL: Installed skills ═══════ */}
        <div className="flex-[3] min-w-0 overflow-y-auto space-y-3 pr-1">

          {/* ── Global Skills Section ── */}
          {globalSkills.length > 0 && (
            <div className={`bg-white dark:bg-[#141414] rounded-xl border border-slate-200/80 dark:border-white/5 shadow-sm overflow-hidden border-l-[3px] ${sourceAccentCls.Global}`}>
              <button
                onClick={() => toggleGroup('_global')}
                className="w-full flex items-center gap-2.5 px-4 py-2.5 text-left hover:bg-slate-50/50 dark:hover:bg-white/[0.02] transition-colors"
              >
                <div className="text-slate-400 transition-transform duration-200" style={{ transform: collapsedGroups.has('_global') ? 'rotate(0deg)' : 'rotate(90deg)' }}>
                  <ChevronRight size={12} />
                </div>
                <span className={`text-[11px] font-bold px-2 py-0.5 rounded-md border ${sourceBadgeCls.Global}`}>
                  Global Skills
                </span>
                <span className="text-[11px] text-slate-400 tabular-nums">{globalSkills.length} skill{globalSkills.length !== 1 ? 's' : ''}</span>
              </button>

              {!collapsedGroups.has('_global') && (
                <div className="border-t border-slate-100/80 dark:border-white/[0.03]">
                  {globalSkills.map((skill, idx) => {
                    const expanded = expandedSkills.has(skill.name);
                    const subLabel = skillSubfolderLabel(skill);
                    return (
                      <div key={skill.name} className={idx < globalSkills.length - 1 ? 'border-b border-slate-100/60 dark:border-white/[0.03]' : ''}>
                        <div
                          className="flex items-center gap-2.5 px-4 py-2.5 cursor-pointer hover:bg-slate-50/80 dark:hover:bg-white/[0.02] transition-colors group"
                          onClick={() => toggleExpanded(skill.name)}
                        >
                          <div className="text-slate-300 dark:text-slate-600 transition-transform duration-200" style={{ transform: expanded ? 'rotate(90deg)' : 'rotate(0deg)' }}>
                            <ChevronRight size={11} />
                          </div>
                          <span className="text-xs font-semibold text-slate-700 dark:text-slate-200 flex-1">{skill.name}</span>
                          <span className={`text-[11px] font-semibold px-1.5 py-0.5 rounded-md border ${sourceBadgeCls[skill.source?.type] || sourceBadgeCls.Compat}`}>
                            {subLabel}
                          </span>
                          {skill.tool_defs && skill.tool_defs.length > 0 && (
                            <span className="inline-flex items-center gap-1 text-[11px] text-slate-400 bg-slate-100/80 dark:bg-white/5 px-1.5 py-0.5 rounded-md">
                              <Wrench size={8} /> {skill.tool_defs.length}
                            </span>
                          )}
                          <div className="flex items-center gap-0.5" onClick={(e) => e.stopPropagation()}>
                            <button
                              onClick={() => uninstallMarketplaceSkill(skill.name, 'global')}
                              disabled={mpUninstalling.has(skill.name)}
                              className="px-1.5 py-0.5 rounded text-[11px] font-medium text-slate-400 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-500/10 border border-transparent hover:border-red-200 dark:hover:border-red-500/20 transition-all disabled:opacity-50"
                              title="Remove"
                            >
                              {mpUninstalling.has(skill.name) ? <RefreshCw size={10} className="animate-spin" /> : <span className="flex items-center gap-1"><Trash2 size={9} /> Remove</span>}
                            </button>
                          </div>
                        </div>

                        {expanded && (
                          <div className="px-4 pb-3 pl-9 space-y-2">
                            <p className="text-[12px] text-slate-500 dark:text-slate-400 leading-relaxed">{skill.description}</p>
                            {skill.tool_defs && skill.tool_defs.length > 0 && (
                              <div className="overflow-x-auto rounded-lg border border-slate-100 dark:border-white/5">
                                <table className="w-full text-[11px]">
                                  <thead>
                                    <tr className="text-left text-slate-400 bg-slate-50/80 dark:bg-white/[0.02]">
                                      <th className="py-1.5 px-2.5 font-bold">Tool</th>
                                      <th className="py-1.5 px-2.5 font-bold">Description</th>
                                      <th className="py-1.5 px-2.5 font-bold">Cmd</th>
                                    </tr>
                                  </thead>
                                  <tbody>
                                    {skill.tool_defs.map((tool) => (
                                      <tr key={tool.name} className="border-t border-slate-100/60 dark:border-white/[0.03]">
                                        <td className="py-1 px-2.5 font-mono font-semibold text-slate-600 dark:text-slate-300">{tool.name}</td>
                                        <td className="py-1 px-2.5 text-slate-500 max-w-40 truncate">{tool.description}</td>
                                        <td className="py-1 px-2.5 font-mono text-slate-400 max-w-40 truncate">{tool.cmd}</td>
                                      </tr>
                                    ))}
                                  </tbody>
                                </table>
                              </div>
                            )}
                            {skill.content && (
                              <details className="text-[11px] group/details">
                                <summary className="text-slate-400 cursor-pointer hover:text-slate-600 dark:hover:text-slate-300 font-medium transition-colors">Content preview</summary>
                                <pre className="mt-1.5 p-2.5 bg-slate-50 dark:bg-black/20 border border-slate-100 dark:border-white/5 rounded-lg text-[10px] overflow-x-auto max-h-32 whitespace-pre-wrap text-slate-600 dark:text-slate-400">
                                  {skill.content.slice(0, 400)}{skill.content.length > 400 ? '...' : ''}
                                </pre>
                              </details>
                            )}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {/* ── Project Skills Section ── */}
          {projectSkills.length > 0 && (
            <div className={`bg-white dark:bg-[#141414] rounded-xl border border-slate-200/80 dark:border-white/5 shadow-sm overflow-hidden border-l-[3px] ${sourceAccentCls.Project}`}>
              <button
                onClick={() => toggleGroup('_project')}
                className="w-full flex items-center gap-2.5 px-4 py-2.5 text-left hover:bg-slate-50/50 dark:hover:bg-white/[0.02] transition-colors"
              >
                <div className="text-slate-400 transition-transform duration-200" style={{ transform: collapsedGroups.has('_project') ? 'rotate(0deg)' : 'rotate(90deg)' }}>
                  <ChevronRight size={12} />
                </div>
                <span className={`text-[11px] font-bold px-2 py-0.5 rounded-md border ${sourceBadgeCls.Project}`}>
                  Project Skills
                </span>
                <span className="text-[11px] text-slate-400 tabular-nums">{projectSkills.length} skill{projectSkills.length !== 1 ? 's' : ''}</span>
              </button>

              {!collapsedGroups.has('_project') && (
                <div className="border-t border-slate-100/80 dark:border-white/[0.03]">
                  {projectSkills.map((skill, idx) => {
                    const expanded = expandedSkills.has(skill.name);
                    const file = skillFiles.find((f) => f.name === skill.name);
                    return (
                      <div key={skill.name} className={idx < projectSkills.length - 1 ? 'border-b border-slate-100/60 dark:border-white/[0.03]' : ''}>
                        <div
                          className="flex items-center gap-2.5 px-4 py-2.5 cursor-pointer hover:bg-slate-50/80 dark:hover:bg-white/[0.02] transition-colors group"
                          onClick={() => toggleExpanded(skill.name)}
                        >
                          <div className="text-slate-300 dark:text-slate-600 transition-transform duration-200" style={{ transform: expanded ? 'rotate(90deg)' : 'rotate(0deg)' }}>
                            <ChevronRight size={11} />
                          </div>
                          <span className="text-xs font-semibold text-slate-700 dark:text-slate-200 flex-1">{skill.name}</span>
                          {skill.tool_defs && skill.tool_defs.length > 0 && (
                            <span className="inline-flex items-center gap-1 text-[11px] text-slate-400 bg-slate-100/80 dark:bg-white/5 px-1.5 py-0.5 rounded-md">
                              <Wrench size={8} /> {skill.tool_defs.length}
                            </span>
                          )}
                          <div className="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
                            {file && (
                              <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                                <button onClick={() => startEditing(skill)} className="p-1 rounded text-slate-400 hover:text-blue-500 hover:bg-blue-50 dark:hover:bg-blue-500/10 transition-all" title="Edit"><Pencil size={10} /></button>
                              </div>
                            )}
                            <button
                              onClick={() => handleMoveToGlobal(skill.name)}
                              disabled={mpMoving.has(skill.name)}
                              className="px-1.5 py-0.5 rounded text-[11px] font-medium text-slate-400 hover:text-indigo-500 hover:bg-indigo-50 dark:hover:bg-indigo-500/10 border border-transparent hover:border-indigo-200 dark:hover:border-indigo-500/20 transition-all disabled:opacity-50"
                              title="Move to Global"
                            >
                              {mpMoving.has(skill.name) ? <RefreshCw size={10} className="animate-spin" /> : <span className="flex items-center gap-1"><ArrowUpRight size={9} /> Global</span>}
                            </button>
                            <button
                              onClick={() => uninstallMarketplaceSkill(skill.name, 'project')}
                              disabled={mpUninstalling.has(skill.name)}
                              className="px-1.5 py-0.5 rounded text-[11px] font-medium text-slate-400 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-500/10 border border-transparent hover:border-red-200 dark:hover:border-red-500/20 transition-all disabled:opacity-50"
                              title="Remove"
                            >
                              {mpUninstalling.has(skill.name) ? <RefreshCw size={10} className="animate-spin" /> : <span className="flex items-center gap-1"><Trash2 size={9} /> Remove</span>}
                            </button>
                          </div>
                        </div>

                        {expanded && (
                          <div className="px-4 pb-3 pl-9 space-y-2">
                            <p className="text-[12px] text-slate-500 dark:text-slate-400 leading-relaxed">{skill.description}</p>
                            {skill.tool_defs && skill.tool_defs.length > 0 && (
                              <div className="overflow-x-auto rounded-lg border border-slate-100 dark:border-white/5">
                                <table className="w-full text-[11px]">
                                  <thead>
                                    <tr className="text-left text-slate-400 bg-slate-50/80 dark:bg-white/[0.02]">
                                      <th className="py-1.5 px-2.5 font-bold">Tool</th>
                                      <th className="py-1.5 px-2.5 font-bold">Description</th>
                                      <th className="py-1.5 px-2.5 font-bold">Cmd</th>
                                    </tr>
                                  </thead>
                                  <tbody>
                                    {skill.tool_defs.map((tool) => (
                                      <tr key={tool.name} className="border-t border-slate-100/60 dark:border-white/[0.03]">
                                        <td className="py-1 px-2.5 font-mono font-semibold text-slate-600 dark:text-slate-300">{tool.name}</td>
                                        <td className="py-1 px-2.5 text-slate-500 max-w-40 truncate">{tool.description}</td>
                                        <td className="py-1 px-2.5 font-mono text-slate-400 max-w-40 truncate">{tool.cmd}</td>
                                      </tr>
                                    ))}
                                  </tbody>
                                </table>
                              </div>
                            )}
                            {skill.content && (
                              <details className="text-[11px] group/details">
                                <summary className="text-slate-400 cursor-pointer hover:text-slate-600 dark:hover:text-slate-300 font-medium transition-colors">Content preview</summary>
                                <pre className="mt-1.5 p-2.5 bg-slate-50 dark:bg-black/20 border border-slate-100 dark:border-white/5 rounded-lg text-[10px] overflow-x-auto max-h-32 whitespace-pre-wrap text-slate-600 dark:text-slate-400">
                                  {skill.content.slice(0, 400)}{skill.content.length > 400 ? '...' : ''}
                                </pre>
                              </details>
                            )}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {globalSkills.length === 0 && projectSkills.length === 0 && (
            <div className="bg-white dark:bg-[#141414] rounded-xl border border-dashed border-slate-200 dark:border-white/10 p-8 text-center">
              <div className="w-10 h-10 rounded-xl bg-slate-100 dark:bg-white/5 flex items-center justify-center mx-auto mb-3">
                <Package size={18} className="text-slate-300 dark:text-slate-600" />
              </div>
              <p className="text-[13px] font-medium text-slate-500 dark:text-slate-400">No skills installed</p>
              <p className="text-[11px] text-slate-400 mt-1">Install skills from the Library</p>
            </div>
          )}
        </div>

        {/* ═══════ RIGHT PANEL: Library ═══════ */}
        <div className="flex-[3] min-w-0 flex flex-col min-h-0">
          <div className="bg-white dark:bg-[#141414] rounded-xl border border-slate-200/80 dark:border-white/5 shadow-sm flex flex-col flex-1 min-h-0 overflow-hidden">
            {/* Sticky header + search */}
            <div className="shrink-0">
              <div className="flex items-center gap-2.5 px-4 py-2.5 bg-gradient-to-r from-blue-50/80 to-transparent dark:from-blue-500/5 dark:to-transparent border-b border-slate-100 dark:border-white/5">
                <div className="w-5 h-5 rounded-md bg-blue-500/10 flex items-center justify-center">
                  <Book size={11} className="text-blue-500" />
                </div>
                <span className="text-xs font-bold text-slate-700 dark:text-slate-200 tracking-wide uppercase">Library</span>
              </div>
              <div className="px-3 py-2 border-b border-slate-100 dark:border-white/5">
                <div className="flex items-center gap-2 bg-slate-50/80 dark:bg-white/[0.03] rounded-lg px-3 py-2 border border-slate-200/50 dark:border-white/5 focus-within:border-blue-300 dark:focus-within:border-blue-500/30 focus-within:ring-2 focus-within:ring-blue-500/10 transition-all">
                  <Search size={13} className="text-slate-400 shrink-0" />
                  <input
                    type="text"
                    value={mpQuery}
                    onChange={(e) => handleMpQueryChange(e.target.value)}
                    placeholder="Search skills..."
                    className="flex-1 text-xs bg-transparent outline-none placeholder:text-slate-400"
                  />
                  {mpQuery && (
                    <button onClick={() => { setMpQuery(''); setMpResults([]); }} className="text-slate-400 hover:text-slate-600 dark:hover:text-slate-300">
                      <X size={12} />
                    </button>
                  )}
                  {mpLoading && (
                    <div className="w-3.5 h-3.5 border-2 border-blue-500/30 border-t-blue-500 rounded-full animate-spin shrink-0" />
                  )}
                </div>
              </div>
              {/* Third-party warning — always visible */}
              <div className="px-3 py-2 border-b border-amber-200/60 dark:border-amber-500/10 bg-amber-50/60 dark:bg-amber-500/[0.04]">
                <div className="flex items-center gap-2">
                  <ShieldAlert size={13} className="text-amber-500 shrink-0" />
                  <p className="text-[12px] font-medium text-amber-700 dark:text-amber-400 leading-snug">
                    Community skills are third-party code — review before installing, use at your own risk.
                  </p>
                </div>
              </div>
            </div>

            {/* Scrollable results: built-in on top, then marketplace */}
            <div className="flex-1 overflow-y-auto min-h-0">
              {(filteredBuiltIn.length > 0 || filteredMpResults.length > 0) ? (
                <div className="p-2 space-y-1">
                  {/* Built-in skills */}
                  {filteredBuiltIn.map((bi) => (
                    <div
                      key={`bi-${bi.name}`}
                      className={`px-3 py-2.5 rounded-lg border transition-all ${
                        bi.installed
                          ? 'bg-emerald-50/40 dark:bg-emerald-500/5 border-emerald-200/50 dark:border-emerald-500/10'
                          : 'bg-blue-50/30 dark:bg-blue-500/5 border-blue-200/50 dark:border-blue-500/10 hover:border-blue-300 dark:hover:border-blue-500/20 hover:shadow-sm'
                      }`}
                    >
                      <div className="flex items-start gap-2.5">
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-1.5">
                            <Sparkles size={10} className="text-blue-500 shrink-0" />
                            <span className="text-xs font-bold text-slate-700 dark:text-slate-200">{bi.name}</span>
                            <span className="text-[11px] font-semibold text-blue-500 bg-blue-500/10 px-1.5 py-0.5 rounded uppercase">Linggen</span>
                          </div>
                          <p className="text-[12px] text-slate-500 dark:text-slate-400 mt-0.5 line-clamp-2 leading-relaxed">{bi.description}</p>
                        </div>
                        <div className="shrink-0 pt-0.5">
                          {bi.installed ? (
                            <button
                              onClick={() => installBuiltInSkill(bi.name)}
                              disabled={biInstalling.has(bi.name)}
                              className="px-2 py-1 text-[11px] font-semibold rounded-md border border-slate-200 dark:border-white/10 text-slate-500 hover:text-blue-600 hover:border-blue-200 dark:hover:border-blue-500/30 hover:bg-blue-50/50 dark:hover:bg-blue-500/5 disabled:opacity-50 transition-all"
                            >
                              {biInstalling.has(bi.name) ? '...' : 'Update'}
                            </button>
                          ) : (
                            <button
                              onClick={() => installBuiltInSkill(bi.name)}
                              disabled={biInstalling.has(bi.name)}
                              className="px-3 py-1 text-[11px] font-semibold rounded-md bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50 shadow-sm shadow-blue-600/20 transition-colors"
                            >
                              {biInstalling.has(bi.name) ? (
                                <span className="inline-flex items-center gap-1">
                                  <div className="w-2.5 h-2.5 border-[1.5px] border-white/30 border-t-white rounded-full animate-spin" />
                                  Installing
                                </span>
                              ) : 'Install'}
                            </button>
                          )}
                        </div>
                      </div>
                    </div>
                  ))}

                  {/* Separator between built-in and community */}
                  {filteredBuiltIn.length > 0 && filteredMpResults.length > 0 && (
                    <div className="flex items-center gap-2 py-1.5 px-1">
                      <div className="flex-1 border-t border-slate-200/60 dark:border-white/5" />
                      <span className="text-[11px] font-semibold text-slate-400 uppercase tracking-wider">
                        {mpQuery.trim() ? 'Community' : 'Trending'}
                      </span>
                      <div className="flex-1 border-t border-slate-200/60 dark:border-white/5" />
                    </div>
                  )}


                  {/* Marketplace / community skills */}
                  {filteredMpResults.map((skill) => {
                    const isInstalled = installedNames.has(skill.name);
                    const isInstalling = mpInstalling.has(skill.name);
                    // Extract owner/repo from GitHub URL for display
                    const repoSlug = skill.url ? skill.url.replace(/^https?:\/\/github\.com\//, '').replace(/\/$/, '') : '';

                    return (
                      <div
                        key={skill.skill_id || skill.name}
                        className={`px-3 py-2.5 rounded-lg border transition-all ${
                          isInstalled
                            ? 'bg-emerald-50/40 dark:bg-emerald-500/5 border-emerald-200/50 dark:border-emerald-500/10'
                            : 'bg-slate-50/50 dark:bg-white/[0.02] border-slate-100 dark:border-white/[0.03] hover:border-slate-200 dark:hover:border-white/10 hover:shadow-sm'
                        }`}
                      >
                        <div className="flex items-start gap-2.5">
                          <div className="flex-1 min-w-0">
                            {/* Row 1: name + source badge + installs */}
                            <div className="flex items-center gap-1.5">
                              <span className="text-xs font-bold text-slate-700 dark:text-slate-200">{skill.name}</span>
                              {skill.source_registry === 'clawhub' ? (
                                <span className="text-[10px] font-semibold text-purple-600 dark:text-purple-400 bg-purple-500/10 px-1 py-0.5 rounded">ClawHub</span>
                              ) : skill.source_registry === 'skills.sh' ? (
                                <span className="text-[10px] font-semibold text-slate-500 dark:text-slate-400 bg-slate-500/10 px-1 py-0.5 rounded">skills.sh</span>
                              ) : null}
                              {skill.install_count > 0 && (
                                <span className="inline-flex items-center gap-0.5 text-[11px] text-slate-400">
                                  <Download size={8} />
                                  {skill.install_count.toLocaleString()}
                                </span>
                              )}
                            </div>
                            {/* Row 2: description */}
                            {skill.description && (
                              <p className="text-[12px] text-slate-500 dark:text-slate-400 mt-0.5 line-clamp-2 leading-relaxed">{skill.description}</p>
                            )}
                            {/* Row 3: metadata line — source repo + updated_at */}
                            <div className="flex items-center gap-2 mt-1 text-[11px] text-slate-400">
                              {skill.url && (
                                <a
                                  href={skill.url}
                                  target="_blank"
                                  rel="noopener noreferrer"
                                  className="inline-flex items-center gap-0.5 hover:text-blue-500 transition-colors truncate max-w-[200px]"
                                  onClick={(e) => e.stopPropagation()}
                                  title={skill.url}
                                >
                                  <ExternalLink size={8} className="shrink-0" />
                                  {repoSlug || 'source'}
                                </a>
                              )}
                              {skill.updated_at && (
                                <>
                                  <span className="text-slate-300 dark:text-white/10">·</span>
                                  <span title={skill.updated_at}>
                                    {formatRelativeDate(skill.updated_at)}
                                  </span>
                                </>
                              )}
                            </div>
                          </div>
                          <div className="shrink-0 pt-0.5">
                            {isInstalled ? (
                              <span className="inline-flex items-center gap-1 px-2 py-1 text-[11px] font-semibold rounded-md text-emerald-600 dark:text-emerald-400 bg-emerald-500/10 border border-emerald-200/50 dark:border-emerald-500/20">
                                <Check size={9} /> Installed
                              </span>
                            ) : (
                              <button
                                onClick={() => installMarketplaceSkill(skill)}
                                disabled={isInstalling}
                                className="px-3 py-1 text-[11px] font-semibold rounded-md bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50 shadow-sm shadow-blue-600/20 transition-colors"
                              >
                                {isInstalling ? (
                                  <span className="inline-flex items-center gap-1">
                                    <div className="w-2.5 h-2.5 border-[1.5px] border-white/30 border-t-white rounded-full animate-spin" />
                                    Installing
                                  </span>
                                ) : 'Install'}
                              </button>
                            )}
                          </div>
                        </div>
                      </div>
                    );
                  })}
                </div>
              ) : (
                !mpLoading && (
                  <div className="flex flex-col items-center justify-center py-12 px-4">
                    <div className="w-12 h-12 rounded-2xl bg-gradient-to-br from-blue-100 to-blue-50 dark:from-blue-500/10 dark:to-blue-500/5 flex items-center justify-center mb-3">
                      <Book size={20} className="text-blue-400" />
                    </div>
                    <p className="text-[13px] font-medium text-slate-500 dark:text-slate-400">
                      {mpQuery ? 'No skills found' : 'Loading skills...'}
                    </p>
                    <p className="text-[11px] text-slate-400 mt-1 text-center max-w-[180px]">
                      {mpQuery
                        ? 'Try a different search term'
                        : 'Fetching community skills'}
                    </p>
                  </div>
                )
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
