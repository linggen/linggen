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
            <div className="view flex h-full items-center justify-center">
                <div className="text-sm text-slate-500">Loading library...</div>
            </div>
        );
    }

    return (
        <div className="view flex h-full flex-col gap-6 overflow-hidden p-6 bg-[var(--bg-content)] text-[var(--text-primary)]">
            <div className="flex flex-col gap-2">
                <div className="relative max-w-xl">
                    <MagnifyingGlassIcon 
                        className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-slate-400" 
                        style={{ width: '16px', height: '16px' }}
                    />
                    <input
                        type="text"
                        placeholder="Search files by name or folder..."
                        value={searchQuery}
                        onChange={e => setSearchQuery(e.target.value)}
                        className="w-full rounded-md border border-slate-300 bg-white py-2.5 pl-10 pr-3 text-sm text-slate-900 placeholder:text-slate-400 shadow-sm outline-none focus:border-indigo-500 focus:ring-2 focus:ring-indigo-500/30 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-100"
                    />
                </div>
                <p className="text-xs text-slate-500 px-1">
                    {filteredPacks.length} / {packs.length} files
                </p>
            </div>

            <div className="flex-1 overflow-auto rounded-lg border border-slate-200 bg-white dark:border-slate-800 dark:bg-slate-900/50 shadow-sm">
                <table className="w-full border-collapse text-sm">
                    <thead className="sticky top-0 z-10 bg-slate-50 dark:bg-slate-800">
                        <tr>
                            <th className="border-b border-slate-200 px-4 py-3 text-left text-xs font-semibold tracking-wider text-slate-600 dark:border-slate-700 dark:text-slate-300">
                                NAME
                            </th>
                            <th className="border-b border-slate-200 px-4 py-3 text-left text-xs font-semibold tracking-wider text-slate-600 dark:border-slate-700 dark:text-slate-300">
                                FOLDER
                            </th>
                            <th className="border-b border-slate-200 px-4 py-3 text-left text-xs font-semibold tracking-wider text-slate-600 dark:border-slate-700 dark:text-slate-300">
                                CREATED TIME
                            </th>
                            <th className="border-b border-slate-200 px-4 py-3 text-left text-xs font-semibold tracking-wider text-slate-600 dark:border-slate-700 dark:text-slate-300">
                                UPDATED TIME
                            </th>
                        </tr>
                    </thead>
                    <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                        {filteredPacks.length > 0 ? (
                            filteredPacks.map(pack => (
                                <tr
                                    key={pack.id}
                                    onClick={() => onSelectPack?.(pack.id)}
                                    className="group cursor-pointer hover:bg-slate-50 dark:hover:bg-slate-800/50 transition-colors"
                                >
                                    <td className="px-4 py-3">
                                        <div className="flex items-center gap-3">
                                            <div className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded bg-indigo-500/10 text-[10px] font-bold text-indigo-600 dark:text-indigo-400 border border-indigo-500/20">
                                                MD
                                            </div>
                                            <div className="min-w-0">
                                                <div className="truncate font-medium text-slate-900 dark:text-slate-100 group-hover:text-indigo-600 dark:group-hover:text-indigo-400">
                                                    {pack.filename || pack.name}
                                                </div>
                                                <div className="truncate text-[11px] text-slate-400">
                                                    {pack.name !== pack.filename ? pack.name : pack.id}
                                                </div>
                                            </div>
                                        </div>
                                    </td>
                                    <td className="px-4 py-3">
                                        <div className="flex items-center gap-2 text-slate-600 dark:text-slate-400">
                                            <FolderIcon className="text-slate-400" style={{ width: '14px', height: '14px' }} />
                                            <span className="truncate text-[11px] font-medium uppercase tracking-wider">
                                                {pack.folder || 'general'}
                                            </span>
                                        </div>
                                    </td>
                                    <td className="px-4 py-3 text-slate-500 dark:text-slate-400 text-xs">
                                        <div className="flex items-center gap-2">
                                            <CalendarIcon className="text-slate-400" style={{ width: '14px', height: '14px' }} />
                                            <span className="truncate">{formatDate(pack.created_at)}</span>
                                        </div>
                                    </td>
                                    <td className="px-4 py-3 text-slate-500 dark:text-slate-400 text-xs">
                                        <div className="flex items-center gap-2">
                                            <ClockIcon className="text-slate-400" style={{ width: '14px', height: '14px' }} />
                                            <span className="truncate">{formatDate(pack.updated_at)}</span>
                                        </div>
                                    </td>
                                </tr>
                            ))
                        ) : (
                            <tr>
                                <td colSpan={4} className="px-6 py-12 text-center text-sm text-slate-500 dark:text-slate-400">
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
