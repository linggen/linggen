import { useState, useEffect } from 'react';
import {
    MagnifyingGlassIcon,
    FolderIcon,
    CalendarIcon,
    ClockIcon
} from '@heroicons/react/24/outline';
import { type LibraryPack, listPacks } from '../api';

interface LibraryViewProps {
    onSelectPack?: (id: string) => void;
}

export function LibraryView({ onSelectPack }: LibraryViewProps) {
    const [packs, setPacks] = useState<LibraryPack[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [searchQuery, setSearchQuery] = useState('');

    useEffect(() => {
        const fetchPacks = async () => {
            try {
                const data = await listPacks();
                setPacks(data);
            } catch (err) {
                console.error('Failed to load packs:', err);
            } finally {
                setIsLoading(false);
            }
        };
        fetchPacks();
    }, []);

    const filteredPacks = packs.filter(pack => {
        const matchesSearch = (pack.filename || '').toLowerCase().includes(searchQuery.toLowerCase()) ||
                             pack.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
                             (pack.folder || '').toLowerCase().includes(searchQuery.toLowerCase());
        return matchesSearch;
    });

    const formatDate = (dateStr?: string) => {
        if (!dateStr) return '-';
        return new Date(dateStr).toLocaleString();
    };

    if (isLoading) {
        return (
            <div className="flex h-full items-center justify-center bg-[var(--bg-content)]">
                <div className="text-sm text-[var(--text-secondary)]">Loading library...</div>
            </div>
        );
    }

    return (
        <div className="flex flex-1 flex-col gap-6 overflow-hidden p-6 bg-[var(--bg-content)] text-[var(--text-primary)]">
            <div className="flex flex-col gap-2">
                <div className="relative max-w-xl">
                    <MagnifyingGlassIcon 
                        className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-[var(--text-secondary)] w-4 h-4" 
                    />
                    <input
                        type="text"
                        placeholder="Search files by name or folder..."
                        value={searchQuery}
                        onChange={e => setSearchQuery(e.target.value)}
                        className="w-full rounded-md border border-[var(--border-color)] bg-[var(--bg-app)] py-2 pl-10 pr-3 text-sm text-[var(--text-primary)] placeholder:text-[var(--text-secondary)] shadow-none outline-none focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)]/30"
                    />
                </div>
                <p className="text-[11px] text-[var(--text-secondary)] px-1 uppercase tracking-wider font-medium">
                    {filteredPacks.length} / {packs.length} files
                </p>
            </div>

            <div className="flex-1 overflow-auto rounded-lg border border-[var(--border-color)] bg-[var(--bg-sidebar)] shadow-none">
                <table className="w-full border-collapse text-sm">
                    <thead className="sticky top-0 z-10 bg-[var(--bg-sidebar)] border-b border-[var(--border-color)]">
                        <tr>
                            <th className="px-4 py-3 text-left text-[11px] font-semibold tracking-wider text-[var(--text-secondary)] uppercase">
                                NAME
                            </th>
                            <th className="px-4 py-3 text-left text-[11px] font-semibold tracking-wider text-[var(--text-secondary)] uppercase">
                                FOLDER
                            </th>
                            <th className="px-4 py-3 text-left text-[11px] font-semibold tracking-wider text-[var(--text-secondary)] uppercase">
                                CREATED TIME
                            </th>
                            <th className="px-4 py-3 text-left text-[11px] font-semibold tracking-wider text-[var(--text-secondary)] uppercase">
                                UPDATED TIME
                            </th>
                        </tr>
                    </thead>
                    <tbody className="divide-y divide-[var(--border-color)]">
                        {filteredPacks.length > 0 ? (
                            filteredPacks.map(pack => (
                                <tr
                                    key={pack.id}
                                    onClick={() => onSelectPack?.(pack.id)}
                                    className="group cursor-pointer hover:bg-[var(--item-hover)] transition-colors"
                                >
                                    <td className="px-4 py-3">
                                        <div className="flex items-center gap-3">
                                            <div className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded bg-[var(--accent)]/10 text-[10px] font-bold text-[var(--accent)] border border-[var(--accent)]/20">
                                                MD
                                            </div>
                                            <div className="min-w-0">
                                                <div className="truncate font-medium text-[var(--text-active)] group-hover:text-[var(--accent)] transition-colors flex items-center gap-2">
                                                    {pack.filename || pack.name}
                                                    {pack.read_only && (
                                                        <span className="bg-[var(--border-color)] px-1.5 py-0.5 rounded text-[9px] uppercase font-bold tracking-wider text-[var(--text-secondary)]">Read Only</span>
                                                    )}
                                                </div>
                                                <div className="truncate text-[11px] text-[var(--text-secondary)] font-mono">
                                                    {pack.name !== pack.filename ? pack.name : pack.id}
                                                </div>
                                            </div>
                                        </div>
                                    </td>
                                    <td className="px-4 py-3">
                                        <div className="flex items-center gap-2 text-[var(--text-secondary)]">
                                            <FolderIcon className="w-3.5 h-3.5 opacity-70" />
                                            <span className="truncate text-[11px] font-medium uppercase tracking-wider">
                                                {pack.folder || 'general'}
                                            </span>
                                        </div>
                                    </td>
                                    <td className="px-4 py-3 text-[var(--text-secondary)] text-[11px]">
                                        <div className="flex items-center gap-2">
                                            <CalendarIcon className="w-3.5 h-3.5 opacity-70" />
                                            <span className="truncate">{formatDate(pack.created_at)}</span>
                                        </div>
                                    </td>
                                    <td className="px-4 py-3 text-[var(--text-secondary)] text-[11px]">
                                        <div className="flex items-center gap-2">
                                            <ClockIcon className="w-3.5 h-3.5 opacity-70" />
                                            <span className="truncate">{formatDate(pack.updated_at)}</span>
                                        </div>
                                    </td>
                                </tr>
                            ))
                        ) : (
                            <tr>
                                <td colSpan={4} className="px-6 py-12 text-center text-sm text-[var(--text-secondary)]">
                                    No files found matching your search.
                                </td>
                            </tr>
                        )}
                    </tbody>
                </table>
            </div>
        </div>
    );
}
