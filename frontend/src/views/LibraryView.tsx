import { useState, useEffect } from 'react';
import MDEditor from '@uiw/react-md-editor';
import {
    MagnifyingGlassIcon,
    ArrowPathIcon,
    ClipboardIcon,
    CheckIcon,
    ArrowLeftIcon,
    TrophyIcon,
    CommandLineIcon,
    DocumentTextIcon,
    ArrowDownTrayIcon
} from '@heroicons/react/24/outline';
import { listRemoteSkills, searchRemoteSkills, type RemoteSkill, downloadSkill, recordSkillInstall, searchSkillsSh, type SkillsShSkill } from '../api';

interface LibraryViewProps {
    onSelectPack?: (id: string | null) => void;
    selectedLibraryPackId?: string | null;
    onRefresh?: () => void;
}

export function LibraryView({ selectedLibraryPackId, onRefresh }: LibraryViewProps) {
    const [remoteSkills, setRemoteSkills] = useState<RemoteSkill[]>([]);
    const [selectedRemoteSkill, setSelectedRemoteSkill] = useState<RemoteSkill | null>(null);
    const [skillsShSkills, setSkillsShSkills] = useState<SkillsShSkill[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [isRefreshing, setIsRefreshing] = useState(false);
    const [searchQuery, setSearchQuery] = useState('');
    const [copiedId, setCopiedId] = useState<string | null>(null);
    const [downloadingId, setDownloadingId] = useState<string | null>(null);
    const [downloadedIds, setDownloadedIds] = useState<Set<string>>(new Set());

    // Pagination state
    const [currentPage, setCurrentPage] = useState(1);
    const [totalPages, setTotalPages] = useState(1);
    const [totalSkills, setTotalSkills] = useState(0);
    const pageSize = 24;
    const isSearching = searchQuery.trim().length > 0;

    const [isDarkMode, setIsDarkMode] = useState(() => {
        if (typeof document === 'undefined') return true;
        const rootTheme = document.documentElement.getAttribute('data-theme');
        if (rootTheme === 'dark') return true;
        if (rootTheme === 'light') return false;
        return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
    });

    useEffect(() => {
        if (typeof document === 'undefined') return;
        const observer = new MutationObserver((mutations) => {
            for (const mutation of mutations) {
                if (mutation.type === 'attributes' && mutation.attributeName === 'data-theme') {
                    const newTheme = document.documentElement.getAttribute('data-theme');
                    if (newTheme === 'dark') setIsDarkMode(true);
                    else if (newTheme === 'light') setIsDarkMode(false);
                    else setIsDarkMode(window.matchMedia('(prefers-color-scheme: dark)').matches);
                }
            }
        });

        observer.observe(document.documentElement, { attributes: true });
        return () => observer.disconnect();
    }, []);

    // Reset selected remote skill when selectedLibraryPackId becomes null
    useEffect(() => {
        if (selectedLibraryPackId === null) {
            setSelectedRemoteSkill(null);
        }
    }, [selectedLibraryPackId]);

    const fetchData = async (page: number, query?: string) => {
        setIsRefreshing(true);
        try {
            // Use search API if query is provided, otherwise list all skills
            const remoteData = query?.trim()
                ? await searchRemoteSkills(query.trim(), page, pageSize)
                : await listRemoteSkills(page, pageSize);

            if (remoteData.success) {
                setRemoteSkills(remoteData.skills);
                setTotalPages(remoteData.pagination.total_pages);
                setTotalSkills(remoteData.pagination.total);
                setCurrentPage(remoteData.pagination.page);
            }
            setSkillsShSkills([]);
        } catch (err) {
            console.error('Failed to load library data:', err);
        } finally {
            setIsLoading(false);
            setIsRefreshing(false);
        }
    };

    const fetchSearchResults = async (query: string) => {
        setIsRefreshing(true);
        try {
            const remoteData = await searchRemoteSkills(query, 1, pageSize);
            if (remoteData.success) {
                setRemoteSkills(remoteData.skills);
                setTotalPages(1);
                setTotalSkills(remoteData.skills.length);
                setCurrentPage(1);
            }

            if (remoteData.success && remoteData.skills.length < 10) {
                const skillsShData = await searchSkillsSh(query, 10);
                setSkillsShSkills(skillsShData.skills || []);
            } else {
                setSkillsShSkills([]);
            }
        } catch (err) {
            console.error('Failed to load search results:', err);
            setRemoteSkills([]);
            setSkillsShSkills([]);
        } finally {
            setIsLoading(false);
            setIsRefreshing(false);
        }
    };

    useEffect(() => {
        fetchData(1);
    }, []);

    // Debounced search for remote skills
    useEffect(() => {
        if (!isSearching) {
            const timeoutId = setTimeout(() => {
                fetchData(1, searchQuery);
            }, 300); // 300ms debounce
            return () => clearTimeout(timeoutId);
        }

        const timeoutId = setTimeout(() => {
            fetchSearchResults(searchQuery.trim());
        }, 300);

        return () => clearTimeout(timeoutId);
    }, [searchQuery]);

    const handlePageChange = (newPage: number) => {
        if (newPage >= 1 && newPage <= totalPages) {
            fetchData(newPage, searchQuery);
            // Scroll to top of skills list
            const container = document.querySelector('.custom-scrollbar');
            if (container) container.scrollTop = 0;
        }
    };

    // Remote skills are already filtered server-side, no need for client-side filtering
    const filteredRemoteSkills = remoteSkills;
    const filteredSkillsSh = skillsShSkills;
    const combinedSearchResults = [
        ...filteredRemoteSkills.map(skill => ({
            source: 'linggen' as const,
            key: `linggen-${skill.skill_id}`,
            remote: skill,
        })),
        ...filteredSkillsSh.map(skill => ({
            source: 'skillsSh' as const,
            key: `skillsSh-${skill.topSource}-${skill.id}`,
            skillsSh: skill,
        })),
    ];

    const getSkillColor = (name: string) => {
        const colors = [
            'from-blue-500/20 to-indigo-500/20 text-blue-400 border-blue-500/30',
            'from-purple-500/20 to-pink-500/20 text-purple-400 border-purple-500/30',
            'from-emerald-500/20 to-teal-500/20 text-emerald-400 border-emerald-500/30',
            'from-orange-500/20 to-amber-500/20 text-orange-400 border-orange-500/30',
            'from-cyan-500/20 to-sky-500/20 text-cyan-400 border-cyan-500/30',
        ];
        let hash = 0;
        for (let i = 0; i < name.length; i++) {
            hash = name.charCodeAt(i) + ((hash << 5) - hash);
        }
        return colors[Math.abs(hash) % colors.length];
    };

    const formatDate = (dateStr?: string) => {
        if (!dateStr) return '-';
        return new Date(dateStr).toLocaleString();
    };

    const getSkillSourceUrl = (skill: RemoteSkill) => {
        // Link to repository root
        return skill.url;
    };

    const handleCopyInstallCommand = (e: React.MouseEvent, command: string, key: string) => {
        e.stopPropagation();
        navigator.clipboard.writeText(command);
        setCopiedId(key);
        setTimeout(() => setCopiedId(null), 2000);
    };

    const handleCopyInstall = (e: React.MouseEvent, skill: RemoteSkill) => {
        const repoName = skill.url.split('/').pop();
        const command = skill.skill === repoName
            ? `linggen skills add ${skill.url}`
            : `linggen skills add ${skill.url} --skill ${skill.skill}`;
        handleCopyInstallCommand(e, command, skill.skill_id);
    };

    const getSkillsShKey = (skill: SkillsShSkill) => `skillsSh-${skill.source}-${skill.id}`;

    const handleCopyInstallSkillsSh = (e: React.MouseEvent, skill: SkillsShSkill) => {
        const repoUrl = `https://github.com/${skill.source}`;
        const skillId = skill.skillId || skill.id.split('/').pop() || skill.id;
        
        // If skillId is the same as the repo name (last part of source), 
        // it might be a single-skill repo.
        const repoName = skill.source.split('/').pop();
        const command = skillId === repoName 
            ? `linggen skills add ${repoUrl}`
            : `linggen skills add ${repoUrl} --skill ${skillId}`;
            
        handleCopyInstallCommand(e, command, getSkillsShKey(skill));
    };

    const handleDownloadSkill = async (e: React.MouseEvent, skill: RemoteSkill) => {
        e.stopPropagation();

        if (downloadingId === skill.skill_id) return;

        setDownloadingId(skill.skill_id);
        try {
            // Check if skill name is the same as repo name to handle single-skill repos
            const repoName = skill.url.split('/').pop();
            const skillName = skill.skill === repoName ? '' : skill.skill;
            
            const result = await downloadSkill(skill.url, skillName, skill.ref || 'main');
            setDownloadedIds(prev => new Set(prev).add(skill.skill_id));

            // Record install in registry (with cooldown protection)
            recordSkillInstall(skill.url, skill.skill, skill.ref || 'main', skill.skill_id, result.content ?? skill.content).catch(err => {
                console.warn('Failed to record install:', err);
            });

            // Refresh library to show the downloaded skill in left sidebar
            onRefresh?.();
            await fetchData(currentPage);

            setTimeout(() => {
                setDownloadedIds(prev => {
                    const next = new Set(prev);
                    next.delete(skill.skill_id);
                    return next;
                });
            }, 3000);
        } catch (error) {
            console.error('Failed to download skill:', error);
            alert(`Failed to download skill: ${error instanceof Error ? error.message : 'Unknown error'}`);
        } finally {
            setDownloadingId(null);
        }
    };

    const handleDownloadSkillsSh = async (e: React.MouseEvent, skill: SkillsShSkill) => {
        e.stopPropagation();
        const key = getSkillsShKey(skill);
        if (downloadingId === key) return;

        setDownloadingId(key);
        try {
            const repoUrl = `https://github.com/${skill.source}`;
            const skillId = skill.skillId || skill.id.split('/').pop() || skill.id;
            
            // Handle single-skill repo case
            const repoName = skill.source.split('/').pop();
            const downloadSkillId = skillId === repoName ? '' : skillId;
            
            const result = await downloadSkill(repoUrl, downloadSkillId, 'main');
            setDownloadedIds(prev => new Set(prev).add(key));

            recordSkillInstall(repoUrl, skillId, 'main', key, result.content).catch(err => {
                console.warn('Failed to record install:', err);
            });

            onRefresh?.();
            await fetchData(currentPage);

            setTimeout(() => {
                setDownloadedIds(prev => {
                    const next = new Set(prev);
                    next.delete(key);
                    return next;
                });
            }, 3000);
        } catch (error) {
            console.error('Failed to download skill:', error);
            alert(`Failed to download skill: ${error instanceof Error ? error.message : 'Unknown error'}`);
        } finally {
            setDownloadingId(null);
        }
    };

    if (isLoading) {
        return (
            <div className="flex h-full items-center justify-center bg-[var(--bg-content)]">
                <div className="flex flex-col items-center gap-3">
                    <ArrowPathIcon className="w-6 h-6 text-[var(--accent)] animate-spin" />
                    <div className="text-sm text-[var(--text-secondary)] font-bold tracking-widest uppercase opacity-50">Loading library...</div>
                </div>
            </div>
        );
    }

    if (selectedRemoteSkill) {
        const skillColor = getSkillColor(selectedRemoteSkill.skill);
        return (
            <div className="flex-1 flex flex-col min-h-0 bg-[var(--bg-content)] text-[var(--text-primary)]">
                {/* Detail Header */}
                <div className="flex-shrink-0 px-8 py-6 border-b border-[var(--border-color)] bg-[var(--bg-sidebar)]/30">
                    <div className="flex items-center justify-between max-w-6xl mx-auto">
                        <div className="flex items-center gap-6">
                            <button
                                onClick={() => setSelectedRemoteSkill(null)}
                                className="p-2.5 hover:bg-[var(--item-hover)] rounded-xl transition-all border border-[var(--border-color)]/50 active:scale-95 shadow-sm"
                            >
                                <ArrowLeftIcon className="w-5 h-5" />
                            </button>
                            <div className="flex items-center gap-5">
                                <div className={`w-16 h-16 rounded-2xl bg-gradient-to-br border flex items-center justify-center text-2xl font-black shadow-lg ${skillColor.split(' ').slice(0, 3).join(' ')}`}>
                                    {selectedRemoteSkill.skill.substring(0, 2).toUpperCase()}
                                </div>
                                <div>
                                    <div className="flex items-center gap-3">
                                        <h2 className="text-3xl font-black text-[var(--text-active)] tracking-tight">
                                            {selectedRemoteSkill.skill}
                                        </h2>
                                        <span className="text-[10px] bg-[var(--accent)]/10 text-[var(--accent)] px-2.5 py-1 rounded-lg border border-[var(--accent)]/20 font-black uppercase tracking-widest shadow-sm">Linggen</span>
                                    </div>
                                    <div className="flex items-center gap-2 mt-1.5">
                                        <p className="text-sm text-[var(--text-secondary)] font-mono opacity-60">{selectedRemoteSkill.url}</p>
                                        <a
                                            href={selectedRemoteSkill.url}
                                            target="_blank"
                                            rel="noopener noreferrer"
                                            className="text-[var(--accent)] hover:text-[var(--accent-hover)] transition-colors"
                                        >
                                            <svg className="w-3.5 h-3.5" fill="currentColor" viewBox="0 0 24 24"><path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" /></svg>
                                        </a>
                                    </div>
                                </div>
                            </div>
                        </div>

                        <div className="flex items-center gap-8">
                            <div className="flex flex-col items-end">
                                <span className="text-[10px] font-black text-[var(--text-muted)] uppercase tracking-widest mb-1">Installs</span>
                                <div className="flex items-center gap-2">
                                    <TrophyIcon className="w-4 h-4 text-yellow-500" />
                                    <span className="text-2xl font-black text-[var(--text-active)] leading-none">{selectedRemoteSkill.install_count}</span>
                                </div>
                            </div>
                        </div>
                    </div>
                </div>

                <div className="flex-1 overflow-y-auto min-h-0 custom-scrollbar">
                    <div className="max-w-6xl mx-auto p-8 grid grid-cols-1 lg:grid-cols-3 gap-8">
                        <div className="lg:col-span-2 flex flex-col gap-8">
                            {/* Install Section */}
                            <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-3xl p-8 shadow-sm flex flex-col gap-5 relative overflow-hidden">
                                <div className="absolute top-0 left-0 w-1 h-full bg-[var(--accent)]"></div>
                                <div className="flex items-center justify-between">
                                    <h3 className="text-xs font-black uppercase tracking-[0.2em] text-[var(--text-secondary)] flex items-center gap-2.5">
                                        <CommandLineIcon className="w-4 h-4 text-[var(--accent)]" />
                                        Installation
                                    </h3>
                                    <span className="text-[10px] font-bold text-[var(--text-muted)] font-mono bg-[var(--bg-app)] px-2 py-1 rounded-lg border border-[var(--border-color)]">
                                        Ref: {selectedRemoteSkill.ref}
                                    </span>
                                </div>
                                <div className="relative group">
                                    <code className="block bg-[var(--bg-app)] p-6 rounded-2xl border-2 border-[var(--border-color)] text-[13px] font-mono text-[var(--accent)] break-all pr-32 shadow-inner leading-relaxed">
                                        {(() => {
                                            const repoName = selectedRemoteSkill.url.split('/').pop();
                                            return selectedRemoteSkill.skill === repoName
                                                ? `linggen skills add ${selectedRemoteSkill.url}`
                                                : `linggen skills add ${selectedRemoteSkill.url} --skill ${selectedRemoteSkill.skill}`;
                                        })()}
                                    </code>
                                    <div className="absolute right-4 top-1/2 -translate-y-1/2 flex items-center gap-2">
                                        <button
                                            onClick={(e) => handleDownloadSkill(e, selectedRemoteSkill)}
                                            disabled={downloadingId === selectedRemoteSkill.skill_id}
                                            className="p-3 bg-[var(--bg-sidebar)] hover:bg-[var(--item-hover)] rounded-xl transition-all border border-[var(--border-color)] shadow-md active:scale-90 group-hover:border-[var(--accent)]/50 disabled:opacity-50 disabled:cursor-not-allowed"
                                            title="Download to library"
                                        >
                                            {downloadingId === selectedRemoteSkill.skill_id ? (
                                                <ArrowPathIcon className="w-5 h-5 text-[var(--accent)] animate-spin" />
                                            ) : downloadedIds.has(selectedRemoteSkill.skill_id) ? (
                                                <CheckIcon className="w-5 h-5 text-green-500" />
                                            ) : (
                                                <ArrowDownTrayIcon className="w-5 h-5 text-[var(--text-secondary)]" />
                                            )}
                                        </button>
                                        <button
                                            onClick={(e) => handleCopyInstall(e, selectedRemoteSkill)}
                                            className="p-3 bg-[var(--bg-sidebar)] hover:bg-[var(--item-hover)] rounded-xl transition-all border border-[var(--border-color)] shadow-md active:scale-90 group-hover:border-[var(--accent)]/50"
                                            title="Copy to clipboard"
                                        >
                                            {copiedId === selectedRemoteSkill.skill_id ? (
                                                <CheckIcon className="w-5 h-5 text-green-500" />
                                            ) : (
                                                <ClipboardIcon className="w-5 h-5 text-[var(--text-secondary)]" />
                                            )}
                                        </button>
                                    </div>
                                </div>
                                <p className="text-[11px] text-[var(--text-muted)] font-medium italic">
                                    Paste this command into your terminal to install this skill locally.
                                </p>
                            </section>

                            {/* Preview Section */}
                            <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-3xl shadow-sm overflow-hidden flex flex-col">
                                <div className="px-8 py-5 border-b border-[var(--border-color)] bg-[var(--item-hover)]/30 flex items-center justify-between">
                                    <div className="flex items-center gap-2.5">
                                        <DocumentTextIcon className="w-4 h-4 text-[var(--accent)]" />
                                        <h3 className="text-xs font-black uppercase tracking-[0.2em] text-[var(--text-secondary)]">Documentation</h3>
                                    </div>
                                    <span className="text-[10px] font-bold text-[var(--text-muted)] uppercase tracking-widest opacity-50">SKILL.md</span>
                                </div>
                                <div className="p-10 bg-[var(--bg-sidebar)]/50" data-color-mode={isDarkMode ? 'dark' : 'light'}>
                                    {selectedRemoteSkill.content ? (
                                        <div className="markdown-preview-container">
                                            <MDEditor.Markdown
                                                source={selectedRemoteSkill.content}
                                                style={{
                                                    backgroundColor: 'transparent',
                                                    color: 'var(--text-primary)',
                                                    fontSize: '15px',
                                                    lineHeight: '1.8'
                                                }}
                                            />
                                        </div>
                                    ) : (
                                        <div className="py-20 text-center flex flex-col items-center gap-4">
                                            <div className="w-16 h-16 rounded-full bg-[var(--bg-app)] flex items-center justify-center border border-[var(--border-color)]">
                                                <DocumentTextIcon className="w-8 h-8 text-[var(--text-muted)] opacity-20" />
                                            </div>
                                            <p className="text-sm text-[var(--text-muted)] italic font-medium">No preview content available for this skill.</p>
                                        </div>
                                    )}
                                </div>
                            </section>
                        </div>

                        <div className="flex flex-col gap-8">
                            {/* Metadata Card */}
                            <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-3xl p-8 shadow-sm flex flex-col gap-6">
                                <h3 className="text-xs font-black uppercase tracking-[0.2em] text-[var(--text-secondary)]">Details</h3>
                                <div className="flex flex-col gap-5">
                                    <div className="flex flex-col gap-1.5">
                                        <span className="text-[10px] font-black text-[var(--text-muted)] uppercase tracking-widest">Repository</span>
                                        <span className="text-xs font-bold text-[var(--text-active)] break-all leading-relaxed">{selectedRemoteSkill.url.replace('https://github.com/', '')}</span>
                                    </div>
                                    <div className="flex flex-col gap-1.5">
                                        <span className="text-[10px] font-black text-[var(--text-muted)] uppercase tracking-widest">Branch / Tag</span>
                                        <span className="text-xs font-mono bg-[var(--bg-app)] px-2 py-1 rounded-lg border border-[var(--border-color)] w-fit">{selectedRemoteSkill.ref}</span>
                                    </div>
                                    <div className="flex flex-col gap-1.5">
                                        <span className="text-[10px] font-black text-[var(--text-muted)] uppercase tracking-widest">Last Indexed</span>
                                        <span className="text-xs font-medium text-[var(--text-secondary)]">{formatDate(selectedRemoteSkill.updated_at)}</span>
                                    </div>
                                </div>
                                <div className="pt-4 border-t border-[var(--border-color)]/50">
                                    <a
                                        href={getSkillSourceUrl(selectedRemoteSkill)}
                                        target="_blank"
                                        rel="noopener noreferrer"
                                        className="flex items-center justify-center gap-2 w-full py-3 rounded-xl bg-[var(--bg-app)] border border-[var(--border-color)] text-[10px] font-black text-[var(--text-active)] uppercase tracking-widest hover:border-[var(--accent)] hover:text-[var(--accent)] transition-all active:scale-95 shadow-sm"
                                    >
                                        Source Code
                                        <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2.5" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" /></svg>
                                    </a>
                                </div>
                            </section>

                            {/* Author/Community Card Placeholder */}
                            <section className="bg-gradient-to-br from-[var(--accent)]/10 to-transparent border border-[var(--accent)]/20 rounded-3xl p-8 shadow-sm flex flex-col gap-4">
                                <div className="w-10 h-10 rounded-full bg-[var(--accent)]/20 flex items-center justify-center text-[var(--accent)]">
                                    <TrophyIcon className="w-5 h-5" />
                                </div>
                                <h3 className="text-sm font-black text-[var(--text-active)]">Linggen Choice</h3>
                                <p className="text-xs text-[var(--text-secondary)] leading-relaxed font-medium">
                                    This skill is part of the Linggen online registry. It has been verified and used by other developers.
                                </p>
                            </section>
                        </div>
                    </div>
                </div>
            </div>
        );
    }

    return (
        <div className="flex-1 flex flex-col min-h-0 gap-6 p-6 bg-[var(--bg-content)] text-[var(--text-primary)]">
            <div className="flex flex-col sm:flex-row justify-between items-start sm:items-center gap-4 flex-shrink-0">
                <div className="flex flex-col gap-3">
                    <h2 className="text-2xl font-black text-[var(--text-active)] tracking-tight">Library</h2>
                </div>
                <div className="flex items-center gap-2">
                    <button
                        onClick={() => {
                            if (isSearching) {
                                fetchSearchResults(searchQuery.trim());
                            } else {
                                fetchData(currentPage, searchQuery);
                            }
                        }}
                        disabled={isRefreshing}
                        className="p-2 hover:bg-[var(--item-hover)] rounded-lg transition-all text-[var(--text-secondary)] hover:text-[var(--text-active)]"
                        title="Refresh library"
                    >
                        <ArrowPathIcon className={`w-4 h-4 ${isRefreshing ? 'animate-spin' : ''}`} />
                    </button>
                    <div className="relative w-64">
                        <MagnifyingGlassIcon
                            className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--text-muted)] w-4 h-4 shrink-0"
                            aria-hidden="true"
                        />
                        <input
                            type="text"
                            placeholder="Search skills..."
                            value={searchQuery}
                            onChange={e => setSearchQuery(e.target.value)}
                            className="w-full rounded-xl border border-[var(--border-color)] bg-[var(--bg-app)] !pl-8 !py-2 pr-3 text-xs text-[var(--text-primary)] placeholder:text-[var(--text-muted)] shadow-none outline-none focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)]/30 transition-all"
                            aria-label="Search skills"
                        />
                    </div>
                </div>
            </div>

            <div className="flex-1 overflow-y-auto min-h-0 flex flex-col gap-8 pr-2 custom-scrollbar">
                {isSearching ? (
                    <section className="flex flex-col gap-6">
                        <div className="flex items-center gap-2 px-1">
                            <TrophyIcon className="w-4 h-4 text-[var(--accent)]" />
                            <h3 className="text-xs font-black uppercase tracking-[0.2em] text-[var(--text-active)]">Search Results</h3>
                            <span className="h-px flex-1 bg-[var(--border-color)] opacity-30"></span>
                            <div className="text-[10px] font-bold text-[var(--text-muted)] uppercase tracking-widest">
                                {combinedSearchResults.length} results
                            </div>
                        </div>

                        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                            {combinedSearchResults.length > 0 ? (
                                combinedSearchResults.map((item) => {
                                    if (item.source === 'linggen' && item.remote) {
                                        const skill = item.remote;
                                        return (
                                            <div
                                                key={item.key}
                                                onClick={() => setSelectedRemoteSkill(skill)}
                                                className="group bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl p-5 cursor-pointer hover:border-[var(--accent)] hover:shadow-lg hover:shadow-[var(--accent)]/5 transition-all relative overflow-hidden"
                                            >
                                                <div className="absolute top-3 right-3 text-[10px] font-black uppercase tracking-widest px-2 py-1 rounded-lg bg-[var(--accent)]/10 text-[var(--accent)] border border-[var(--accent)]/20">
                                                    Linggen
                                                </div>
                                                <div className="flex flex-col gap-3">
                                                    <div className="flex items-center gap-3">
                                                        <div className="w-10 h-10 rounded-xl bg-[var(--accent)]/10 border border-[var(--accent)]/20 flex items-center justify-center text-[var(--accent)] font-black text-xs">
                                                            {skill.skill.substring(0, 2).toUpperCase()}
                                                        </div>
                                                        <div className="flex-1 min-w-0">
                                                            <h4 className="text-sm font-bold text-[var(--text-active)] truncate group-hover:text-[var(--accent)] transition-colors">
                                                                {skill.skill}
                                                            </h4>
                                                            <p className="text-[var(--text-muted)] text-[10px] font-mono truncate">{skill.url.replace('https://github.com/', '')}</p>
                                                        </div>
                                                    </div>
                                                    <div className="flex items-center justify-between mt-2">
                                                        <div className="flex items-center gap-1.5">
                                                            <div className="flex -space-x-1">
                                                                {[1, 2, 3].map(i => (
                                                                    <div key={i} className="w-4 h-4 rounded-full border border-[var(--bg-sidebar)] bg-[var(--border-color)] flex items-center justify-center overflow-hidden">
                                                                        <div className="w-full h-full bg-gradient-to-br from-gray-400 to-gray-600 opacity-50"></div>
                                                                    </div>
                                                                ))}
                                                            </div>
                                                            <span className="text-[10px] font-bold text-[var(--text-secondary)]">
                                                                {skill.install_count} installs
                                                            </span>
                                                        </div>
                                                        <div className="flex items-center gap-1">
                                                            <button
                                                                onClick={(e) => handleDownloadSkill(e, skill)}
                                                                disabled={downloadingId === skill.skill_id}
                                                                className="p-1.5 hover:bg-[var(--bg-app)] rounded-lg transition-all border border-transparent hover:border-[var(--border-color)] disabled:opacity-50 disabled:cursor-not-allowed"
                                                                title="Download to library"
                                                            >
                                                                {downloadingId === skill.skill_id ? (
                                                                    <ArrowPathIcon className="w-3.5 h-3.5 text-[var(--accent)] animate-spin" />
                                                                ) : downloadedIds.has(skill.skill_id) ? (
                                                                    <CheckIcon className="w-3.5 h-3.5 text-green-500" />
                                                                ) : (
                                                                    <ArrowDownTrayIcon className="w-3.5 h-3.5 text-[var(--text-muted)]" />
                                                                )}
                                                            </button>
                                                            <button
                                                                onClick={(e) => handleCopyInstall(e, skill)}
                                                                className="p-1.5 hover:bg-[var(--bg-app)] rounded-lg transition-all border border-transparent hover:border-[var(--border-color)]"
                                                                title="Copy install command"
                                                            >
                                                                {copiedId === skill.skill_id ? (
                                                                    <CheckIcon className="w-3.5 h-3.5 text-green-500" />
                                                                ) : (
                                                                    <ClipboardIcon className="w-3.5 h-3.5 text-[var(--text-muted)]" />
                                                                )}
                                                            </button>
                                                        </div>
                                                    </div>
                                                </div>
                                            </div>
                                        );
                                    }

                                    if (item.source === 'skillsSh' && item.skillsSh) {
                                        const skill = item.skillsSh;
                                        const key = getSkillsShKey(skill);
                                        return (
                                            <div
                                                key={item.key}
                                                className="group bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl p-5 transition-all relative overflow-hidden hover:border-[var(--accent)] hover:shadow-lg hover:shadow-[var(--accent)]/5"
                                            >
                                                <div className="absolute top-3 right-3 text-[10px] font-black uppercase tracking-widest px-2 py-1 rounded-lg bg-[var(--accent)]/10 text-[var(--accent)] border border-[var(--accent)]/20">
                                                    GitHub
                                                </div>
                                                <div className="flex flex-col gap-3">
                                                    <div className="flex items-center gap-3">
                                                        <div className="w-10 h-10 rounded-xl bg-[var(--accent)]/10 border border-[var(--accent)]/20 flex items-center justify-center text-[var(--accent)] font-black text-xs">
                                                            {skill.id.substring(0, 2).toUpperCase()}
                                                        </div>
                                                        <div className="flex-1 min-w-0">
                                                            <a
                                                                href={`https://github.com/${skill.source}`}
                                                                target="_blank"
                                                                rel="noopener noreferrer"
                                                                onClick={(e) => e.stopPropagation()}
                                                                className="text-sm font-bold text-[var(--text-active)] truncate group-hover:text-[var(--accent)] transition-colors block"
                                                                title="Open GitHub repository"
                                                            >
                                                                {skill.name || skill.skillId || skill.id}
                                                            </a>
                                                            <p className="text-[var(--text-muted)] text-[10px] font-mono truncate">{skill.source}</p>
                                                        </div>
                                                    </div>
                                                    <div className="flex items-center justify-between mt-2">
                                                        <div className="flex items-center gap-1.5">
                                                            <div className="flex -space-x-1">
                                                                {[1, 2, 3].map(i => (
                                                                    <div key={i} className="w-4 h-4 rounded-full border border-[var(--bg-sidebar)] bg-[var(--border-color)] flex items-center justify-center overflow-hidden">
                                                                        <div className="w-full h-full bg-gradient-to-br from-gray-400 to-gray-600 opacity-50"></div>
                                                                    </div>
                                                                ))}
                                                            </div>
                                                            <span className="text-[10px] font-bold text-[var(--text-secondary)]">
                                                                {skill.installs} installs
                                                            </span>
                                                        </div>
                                                        <div className="flex items-center gap-1">
                                                            <button
                                                                onClick={(e) => handleDownloadSkillsSh(e, skill)}
                                                                disabled={downloadingId === key}
                                                                className="p-1.5 hover:bg-[var(--bg-app)] rounded-lg transition-all border border-transparent hover:border-[var(--border-color)] disabled:opacity-50 disabled:cursor-not-allowed"
                                                                title="Download to library"
                                                            >
                                                                {downloadingId === key ? (
                                                                    <ArrowPathIcon className="w-3.5 h-3.5 text-[var(--accent)] animate-spin" />
                                                                ) : downloadedIds.has(key) ? (
                                                                    <CheckIcon className="w-3.5 h-3.5 text-green-500" />
                                                                ) : (
                                                                    <ArrowDownTrayIcon className="w-3.5 h-3.5 text-[var(--text-muted)]" />
                                                                )}
                                                            </button>
                                                            <button
                                                                onClick={(e) => handleCopyInstallSkillsSh(e, skill)}
                                                                className="p-1.5 hover:bg-[var(--bg-app)] rounded-lg transition-all border border-transparent hover:border-[var(--border-color)]"
                                                                title="Copy install command"
                                                            >
                                                                {copiedId === key ? (
                                                                    <CheckIcon className="w-3.5 h-3.5 text-green-500" />
                                                                ) : (
                                                                    <ClipboardIcon className="w-3.5 h-3.5 text-[var(--text-muted)]" />
                                                                )}
                                                            </button>
                                                        </div>
                                                    </div>
                                                </div>
                                            </div>
                                        );
                                    }

                                    return null;
                                })
                            ) : (
                                <div className="col-span-full py-8 text-center text-xs text-[var(--text-muted)] italic bg-[var(--bg-sidebar)]/50 border border-dashed border-[var(--border-color)] rounded-2xl">
                                    No skills found matching your search.
                                </div>
                            )}
                        </div>
                    </section>
                ) : (
                    <>
                        {/* Remote Skills Leaderboard */}
                        <section className="flex flex-col gap-4">
                            <div className="flex items-center gap-2 px-1">
                                <TrophyIcon className="w-4 h-4 text-yellow-500" />
                                <h3 className="text-xs font-black uppercase tracking-[0.2em] text-[var(--text-active)]">Linggen Online Skills</h3>
                                <span className="h-px flex-1 bg-[var(--border-color)] opacity-30"></span>
                                <div className="text-[10px] font-bold text-[var(--text-muted)] uppercase tracking-widest">
                                    {totalSkills} total
                                </div>
                            </div>

                            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                                {filteredRemoteSkills.length > 0 ? (
                                    filteredRemoteSkills.map((skill, index) => {
                                        const globalIndex = (currentPage - 1) * pageSize + index;
                                        return (
                                            <div
                                                key={skill.skill_id}
                                                onClick={() => setSelectedRemoteSkill(skill)}
                                                className="group bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl p-5 cursor-pointer hover:border-[var(--accent)] hover:shadow-lg hover:shadow-[var(--accent)]/5 transition-all relative overflow-hidden"
                                            >
                                                {globalIndex < 3 && currentPage === 1 && (
                                                    <div className="absolute top-0 right-0 w-12 h-12 bg-[var(--accent)]/10 flex items-center justify-center rounded-bl-2xl border-b border-l border-[var(--accent)]/20">
                                                        <span className="text-[10px] font-black text-[var(--accent)]">#{globalIndex + 1}</span>
                                                    </div>
                                                )}
                                                <div className="flex flex-col gap-3">
                                                    <div className="flex items-center gap-3">
                                                        <div className="w-10 h-10 rounded-xl bg-[var(--accent)]/10 border border-[var(--accent)]/20 flex items-center justify-center text-[var(--accent)] font-black text-xs">
                                                            {skill.skill.substring(0, 2).toUpperCase()}
                                                        </div>
                                                        <div className="flex-1 min-w-0">
                                                            <h4 className="text-sm font-bold text-[var(--text-active)] truncate group-hover:text-[var(--accent)] transition-colors">
                                                                {skill.skill}
                                                            </h4>
                                                            <p className="text-[10px] text-[var(--text-muted)] font-mono truncate">{skill.url.replace('https://github.com/', '')}</p>
                                                        </div>
                                                    </div>
                                                    <div className="flex items-center justify-between mt-2">
                                                        <div className="flex items-center gap-1.5">
                                                            <div className="flex -space-x-1">
                                                                {[1, 2, 3].map(i => (
                                                                    <div key={i} className="w-4 h-4 rounded-full border border-[var(--bg-sidebar)] bg-[var(--border-color)] flex items-center justify-center overflow-hidden">
                                                                        <div className="w-full h-full bg-gradient-to-br from-gray-400 to-gray-600 opacity-50"></div>
                                                                    </div>
                                                                ))}
                                                            </div>
                                                            <span className="text-[10px] font-bold text-[var(--text-secondary)]">
                                                                {skill.install_count} installs
                                                            </span>
                                                        </div>
                                                        <div className="flex items-center gap-1">
                                                            <button
                                                                onClick={(e) => handleDownloadSkill(e, skill)}
                                                                disabled={downloadingId === skill.skill_id}
                                                                className="p-1.5 hover:bg-[var(--bg-app)] rounded-lg transition-all border border-transparent hover:border-[var(--border-color)] disabled:opacity-50 disabled:cursor-not-allowed"
                                                                title="Download to library"
                                                            >
                                                                {downloadingId === skill.skill_id ? (
                                                                    <ArrowPathIcon className="w-3.5 h-3.5 text-[var(--accent)] animate-spin" />
                                                                ) : downloadedIds.has(skill.skill_id) ? (
                                                                    <CheckIcon className="w-3.5 h-3.5 text-green-500" />
                                                                ) : (
                                                                    <ArrowDownTrayIcon className="w-3.5 h-3.5 text-[var(--text-muted)]" />
                                                                )}
                                                            </button>
                                                            <button
                                                                onClick={(e) => handleCopyInstall(e, skill)}
                                                                className="p-1.5 hover:bg-[var(--bg-app)] rounded-lg transition-all border border-transparent hover:border-[var(--border-color)]"
                                                                title="Copy install command"
                                                            >
                                                                {copiedId === skill.skill_id ? (
                                                                    <CheckIcon className="w-3.5 h-3.5 text-green-500" />
                                                                ) : (
                                                                    <ClipboardIcon className="w-3.5 h-3.5 text-[var(--text-muted)]" />
                                                                )}
                                                            </button>
                                                        </div>
                                                    </div>
                                                </div>
                                            </div>
                                        );
                                    })
                                ) : (
                                    <div className="col-span-full py-12 text-center text-xs text-[var(--text-muted)] italic bg-[var(--bg-sidebar)]/50 border border-dashed border-[var(--border-color)] rounded-2xl">
                                        No remote skills found matching your search.
                                    </div>
                                )}
                            </div>
                        </section>

                        {/* Pagination Controls */}
                        {totalPages > 1 && (
                            <div className="flex items-center justify-center gap-2 py-4 flex-shrink-0">
                                <button
                                    onClick={() => handlePageChange(currentPage - 1)}
                                    disabled={currentPage === 1 || isRefreshing}
                                    className="btn-secondary !px-4 !py-2 disabled:opacity-30 active:scale-95 transition-all"
                                >
                                    Previous
                                </button>

                                <div className="flex items-center gap-1 px-4">
                                    <span className="text-xs font-black text-[var(--text-active)]">{currentPage}</span>
                                    <span className="text-xs font-bold text-[var(--text-muted)]">/</span>
                                    <span className="text-xs font-bold text-[var(--text-muted)]">{totalPages}</span>
                                </div>

                                <button
                                    onClick={() => handlePageChange(currentPage + 1)}
                                    disabled={currentPage === totalPages || isRefreshing}
                                    className="btn-secondary !px-4 !py-2 disabled:opacity-30 active:scale-95 transition-all"
                                >
                                    Next
                                </button>
                            </div>
                        )}
                    </>
                )}
            </div>
        </div>
    );
}
