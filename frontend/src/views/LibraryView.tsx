import { useState, useEffect } from 'react';
import MDEditor from '@uiw/react-md-editor';
import {
    MagnifyingGlassIcon,
    FolderIcon,
    CalendarIcon,
    ClockIcon,
    ArrowPathIcon,
    ClipboardIcon,
    CheckIcon,
    ArrowLeftIcon,
    TrophyIcon,
    CommandLineIcon,
    DocumentTextIcon
} from '@heroicons/react/24/outline';
import { type LibraryPack, getLibrary, listRemoteSkills, type RemoteSkill } from '../api';

interface LibraryViewProps {
    onSelectPack?: (id: string | null) => void;
    selectedLibraryPackId?: string | null;
}

export function LibraryView({ onSelectPack, selectedLibraryPackId }: LibraryViewProps) {
    const [activeTab, setActiveTab] = useState<'community' | 'local'>('community');
    const [localPacks, setLocalPacks] = useState<LibraryPack[]>([]);
    const [remoteSkills, setRemoteSkills] = useState<RemoteSkill[]>([]);
    const [selectedRemoteSkill, setSelectedRemoteSkill] = useState<RemoteSkill | null>(null);
    const [isLoading, setIsLoading] = useState(true);
    const [isRefreshing, setIsRefreshing] = useState(false);
    const [searchQuery, setSearchQuery] = useState('');
    const [copiedId, setCopiedId] = useState<string | null>(null);

    // Pagination state
    const [currentPage, setCurrentPage] = useState(1);
    const [totalPages, setTotalPages] = useState(1);
    const [totalSkills, setTotalSkills] = useState(0);
    const pageSize = 24;

    const [isDarkMode, setIsDarkMode] = useState(() => {
        const rootTheme = document.documentElement.getAttribute('data-theme');
        if (rootTheme === 'dark') return true;
        if (rootTheme === 'light') return false;
        return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
    });

    useEffect(() => {
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

    const fetchData = async (page: number) => {
        setIsRefreshing(true);
        try {
            const [localData, remoteData] = await Promise.all([
                getLibrary(),
                listRemoteSkills(page, pageSize)
            ]);
            
            setLocalPacks(localData.packs);
            
            if (remoteData.success) {
                setRemoteSkills(remoteData.skills);
                setTotalPages(remoteData.pagination.total_pages);
                setTotalSkills(remoteData.pagination.total);
                setCurrentPage(remoteData.pagination.page);
            }
        } catch (err) {
            console.error('Failed to load library data:', err);
        } finally {
            setIsLoading(false);
            setIsRefreshing(false);
        }
    };

    useEffect(() => {
        fetchData(1);
    }, []);

    const handlePageChange = (newPage: number) => {
        if (newPage >= 1 && newPage <= totalPages) {
            fetchData(newPage);
            // Scroll to top of skills list
            const container = document.querySelector('.custom-scrollbar');
            if (container) container.scrollTop = 0;
        }
    };

    const filteredLocalPacks = localPacks.filter(pack => {
        const query = searchQuery.toLowerCase();
        return (pack.filename || '').toLowerCase().includes(query) ||
            pack.name.toLowerCase().includes(query) ||
            (pack.folder || '').toLowerCase().includes(query);
    });

    const filteredRemoteSkills = remoteSkills.filter(skill => {
        const query = searchQuery.toLowerCase();
        return skill.skill.toLowerCase().includes(query) ||
            skill.url.toLowerCase().includes(query);
    });

    const formatDate = (dateStr?: string) => {
        if (!dateStr) return '-';
        return new Date(dateStr).toLocaleString();
    };

    const handleCopyInstall = (e: React.MouseEvent, skill: RemoteSkill) => {
        e.stopPropagation();
        const command = `linggen skills add ${skill.url} --skill ${skill.skill}`;
        navigator.clipboard.writeText(command);
        setCopiedId(skill.skill_id);
        setTimeout(() => setCopiedId(null), 2000);
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
        return (
            <div className="flex-1 flex flex-col min-h-0 gap-6 p-8 bg-[var(--bg-content)] text-[var(--text-primary)]">
                <div className="flex items-center justify-between flex-shrink-0">
                    <div className="flex items-center gap-4">
                        <button
                            onClick={() => setSelectedRemoteSkill(null)}
                            className="p-2.5 hover:bg-[var(--item-hover)] rounded-xl transition-all border border-transparent hover:border-[var(--border-color)] active:scale-95"
                        >
                            <ArrowLeftIcon className="w-5 h-5" />
                        </button>
                        <div>
                            <div className="flex items-center gap-3">
                                <h2 className="text-2xl font-black text-[var(--text-active)] tracking-tight">
                                    {selectedRemoteSkill.skill}
                                </h2>
                                <span className="text-[10px] bg-[var(--accent)]/10 text-[var(--accent)] px-2.5 py-1 rounded-lg border border-[var(--accent)]/20 font-black uppercase tracking-wider">Remote Skill</span>
                            </div>
                            <p className="text-sm text-[var(--text-secondary)] font-mono opacity-60 mt-0.5">{selectedRemoteSkill.url}</p>
                        </div>
                    </div>
                    
                    <div className="flex items-center gap-3">
                        <div className="flex flex-col items-end">
                            <span className="text-[10px] font-black text-[var(--text-muted)] uppercase tracking-widest">Total Installs</span>
                            <span className="text-xl font-black text-[var(--accent)] leading-none">{selectedRemoteSkill.install_count}</span>
                        </div>
                    </div>
                </div>

                <div className="flex-1 overflow-y-auto min-h-0 pr-2 custom-scrollbar flex flex-col gap-6">
                    <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl p-6 shadow-sm flex flex-col gap-4 flex-shrink-0">
                        <div className="flex items-center justify-between">
                            <h3 className="text-[11px] font-black uppercase tracking-[0.2em] text-[var(--text-secondary)] flex items-center gap-2">
                                <CommandLineIcon className="w-4 h-4 text-[var(--accent)]" />
                                Install via CLI
                            </h3>
                            <span className="text-[10px] font-bold text-[var(--text-muted)] font-mono opacity-50">Ref: {selectedRemoteSkill.ref}</span>
                        </div>
                        <div className="relative group">
                            <code className="block bg-[var(--bg-app)] p-5 rounded-xl border-2 border-[var(--border-color)] text-sm font-mono text-[var(--accent)] break-all pr-14 shadow-inner">
                                linggen skills add {selectedRemoteSkill.url} --skill {selectedRemoteSkill.skill}
                            </code>
                            <button
                                onClick={(e) => handleCopyInstall(e, selectedRemoteSkill)}
                                className="absolute right-3 top-1/2 -translate-y-1/2 p-2.5 bg-[var(--bg-sidebar)] hover:bg-[var(--item-hover)] rounded-lg transition-all border border-[var(--border-color)] shadow-sm active:scale-90"
                                title="Copy to clipboard"
                            >
                                {copiedId === selectedRemoteSkill.skill_id ? (
                                    <CheckIcon className="w-4.5 h-4.5 text-green-500" />
                                ) : (
                                    <ClipboardIcon className="w-4.5 h-4.5 text-[var(--text-secondary)]" />
                                )}
                            </button>
                        </div>
                    </section>

                    <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl shadow-sm overflow-hidden flex flex-col flex-shrink-0">
                        <div className="px-6 py-4 border-b border-[var(--border-color)] bg-[var(--item-hover)]/30 flex items-center gap-2">
                            <DocumentTextIcon className="w-4 h-4 text-[var(--accent)]" />
                            <h3 className="text-[11px] font-black uppercase tracking-[0.2em] text-[var(--text-secondary)]">Skill.md Preview</h3>
                        </div>
                        <div className="p-8 bg-[var(--bg-sidebar)]" data-color-mode={isDarkMode ? 'dark' : 'light'}>
                            {selectedRemoteSkill.content ? (
                                <div className="markdown-preview-container">
                                    <MDEditor.Markdown 
                                        source={selectedRemoteSkill.content} 
                                        style={{ 
                                            backgroundColor: 'transparent',
                                            color: 'var(--text-primary)',
                                            fontSize: '14px',
                                            lineHeight: '1.6'
                                        }}
                                    />
                                </div>
                            ) : (
                                <div className="py-12 text-center text-sm text-[var(--text-muted)] italic">
                                    No preview content available for this skill.
                                </div>
                            )}
                        </div>
                    </section>
                    
                    <div className="flex justify-between items-center px-2 pb-4 flex-shrink-0">
                        <span className="text-[10px] font-bold text-[var(--text-muted)] uppercase tracking-widest">
                            Last Updated: {formatDate(selectedRemoteSkill.updated_at)}
                        </span>
                        <a 
                            href={`${selectedRemoteSkill.url}/tree/${selectedRemoteSkill.ref}/${selectedRemoteSkill.skill}`} 
                            target="_blank" 
                            rel="noopener noreferrer"
                            className="text-[10px] font-black text-[var(--accent)] uppercase tracking-widest hover:underline"
                        >
                            View on GitHub ↗
                        </a>
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
                    <div className="flex bg-[var(--bg-app)] p-1 rounded-xl border border-[var(--border-color)] w-fit shadow-inner">
                        <button
                            onClick={() => setActiveTab('community')}
                            className={`px-4 py-1.5 rounded-lg text-[10px] font-black uppercase tracking-widest transition-all ${activeTab === 'community' ? 'bg-[var(--accent)] text-white shadow-md' : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'}`}
                        >
                            Community
                        </button>
                        <button
                            onClick={() => setActiveTab('local')}
                            className={`px-4 py-1.5 rounded-lg text-[10px] font-black uppercase tracking-widest transition-all ${activeTab === 'local' ? 'bg-[var(--accent)] text-white shadow-md' : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'}`}
                        >
                            Local
                        </button>
                    </div>
                </div>
                <div className="flex items-center gap-2">
                    <button
                        onClick={() => fetchData(currentPage)}
                        disabled={isRefreshing}
                        className="p-2 hover:bg-[var(--item-hover)] rounded-lg transition-all text-[var(--text-secondary)] hover:text-[var(--text-active)]"
                        title="Refresh library"
                    >
                        <ArrowPathIcon className={`w-4 h-4 ${isRefreshing ? 'animate-spin' : ''}`} />
                    </button>
                    <div className="relative w-64">
                        <MagnifyingGlassIcon
                            className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--text-secondary)] w-3.5 h-3.5"
                        />
                        <input
                            type="text"
                            placeholder="Search skills..."
                            value={searchQuery}
                            onChange={e => setSearchQuery(e.target.value)}
                            className="w-full rounded-xl border border-[var(--border-color)] bg-[var(--bg-app)] py-1.5 pl-9 pr-3 text-xs text-[var(--text-primary)] placeholder:text-[var(--text-secondary)] shadow-none outline-none focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)]/30 transition-all"
                        />
                    </div>
                </div>
            </div>

            <div className="flex-1 overflow-y-auto min-h-0 flex flex-col gap-8 pr-2 custom-scrollbar">
                {activeTab === 'community' ? (
                    <>
                        {/* Remote Skills Leaderboard */}
                        <section className="flex flex-col gap-4">
                            <div className="flex items-center gap-2 px-1">
                                <TrophyIcon className="w-4 h-4 text-yellow-500" />
                                <h3 className="text-xs font-black uppercase tracking-[0.2em] text-[var(--text-active)]">Community Skills</h3>
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
                                                        <button
                                                            onClick={(e) => handleCopyInstall(e, skill)}
                                                            className="p-1.5 hover:bg-[var(--bg-app)] rounded-lg transition-all border border-transparent hover:border-[var(--border-color)]"
                                                            title="Copy install command"
                                                        >
                                                            {copiedId === skill.skill_id ? (
                                                                <CheckIcon className="w-3.5 h-3.5 text-green-500" />
                                                            ) : (
                                                                <CommandLineIcon className="w-3.5 h-3.5 text-[var(--text-muted)]" />
                                                            )}
                                                        </button>
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
                ) : (
                    /* Local Files Table */
                    <section className="flex flex-col gap-4">
                        <div className="flex items-center gap-2 px-1">
                            <FolderIcon className="w-4 h-4 text-blue-500" />
                            <h3 className="text-xs font-black uppercase tracking-[0.2em] text-[var(--text-active)]">Local Library Files</h3>
                            <span className="h-px flex-1 bg-[var(--border-color)] opacity-30"></span>
                        </div>

                        <div className="overflow-hidden rounded-2xl border border-[var(--border-color)] bg-[var(--bg-sidebar)] shadow-sm">
                            <table className="w-full border-collapse text-left text-xs">
                                <thead>
                                    <tr className="bg-[var(--item-hover)]/30 border-b border-[var(--border-color)]">
                                        <th className="px-6 py-3 font-black text-[var(--text-secondary)] uppercase tracking-widest">Name</th>
                                        <th className="px-6 py-3 font-black text-[var(--text-secondary)] uppercase tracking-widest">Folder</th>
                                        <th className="px-6 py-3 font-black text-[var(--text-secondary)] uppercase tracking-widest hidden md:table-cell">Updated</th>
                                    </tr>
                                </thead>
                                <tbody className="divide-y divide-[var(--border-color)]/50">
                                    {filteredLocalPacks.length > 0 ? (
                                        filteredLocalPacks.map(pack => (
                                            <tr
                                                key={pack.id}
                                                onClick={() => onSelectPack?.(pack.id)}
                                                className="group cursor-pointer hover:bg-[var(--item-hover)]/50 transition-colors"
                                            >
                                                <td className="px-6 py-4">
                                                    <div className="flex items-center gap-3">
                                                        <div className="flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-lg bg-[var(--accent)]/10 text-[9px] font-black text-[var(--accent)] border border-[var(--accent)]/20">
                                                            MD
                                                        </div>
                                                        <div className="min-w-0">
                                                            <div className="font-bold text-[var(--text-active)] group-hover:text-[var(--accent)] transition-colors flex items-center gap-2">
                                                                {pack.filename || pack.name}
                                                                {pack.read_only && (
                                                                    <span className="bg-[var(--border-color)] px-1.5 py-0.5 rounded-[4px] text-[8px] uppercase font-black tracking-tighter text-[var(--text-secondary)]">System</span>
                                                                )}
                                                            </div>
                                                            <div className="truncate text-[10px] text-[var(--text-muted)] font-mono opacity-60">
                                                                {pack.name !== pack.filename ? pack.name : pack.id}
                                                            </div>
                                                        </div>
                                                    </div>
                                                </td>
                                                <td className="px-6 py-4">
                                                    <div className="flex items-center gap-2 text-[var(--text-secondary)]">
                                                        <FolderIcon className="w-3 h-3 opacity-50" />
                                                        <span className="text-[10px] font-bold uppercase tracking-wider opacity-80">
                                                            {pack.folder || 'general'}
                                                        </span>
                                                    </div>
                                                </td>
                                                <td className="px-6 py-4 text-[var(--text-muted)] text-[10px] hidden md:table-cell">
                                                    <div className="flex items-center gap-2">
                                                        <ClockIcon className="w-3 h-3 opacity-50" />
                                                        <span>{formatDate(pack.updated_at)}</span>
                                                    </div>
                                                </td>
                                            </tr>
                                        ))
                                    ) : (
                                        <tr>
                                            <td colSpan={3} className="px-6 py-12 text-center text-xs text-[var(--text-muted)] italic">
                                                No local files found matching your search.
                                            </td>
                                        </tr>
                                    )}
                                </tbody>
                            </table>
                        </div>
                    </section>
                )}
            </div>
        </div>
    );
}
