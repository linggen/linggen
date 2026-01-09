import React, { useEffect, useState } from 'react';
import { type Resource, type IndexMode } from '../api';
import { GraphView } from '../components/GraphView';
import { CM6Editor } from '../components/CM6Editor';

// Helper function to format bytes into human-readable size
function formatSize(bytes: number): string {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

interface WorkspaceViewProps {
    sourceId: string;
    source?: Resource;
    onIndexComplete?: () => void;
    // New props for indexing
    onIndexResource?: (resource: Resource, mode?: IndexMode) => void;
    indexingResourceId?: string | null;
    indexingProgress?: string | null;
    onUpdateSource?: (id: string, name: string, include: string[], exclude: string[]) => Promise<void>;
    selectedNotePath?: string | null;
    selectedMemoryPath?: string | null;
    selectedLibraryPackId?: string | null;
}

export const WorkspaceView: React.FC<WorkspaceViewProps> = ({
    sourceId,
    source: initialSource,
    onIndexResource,
    indexingResourceId,
    indexingProgress,
    onUpdateSource,
    selectedNotePath,
    selectedMemoryPath,
    selectedLibraryPackId
}) => {
    // We don't need local state for source if it's passed as prop. 
    // If we were fetching it ourselves, we might need it, but typically we rely on the prop.
    // If the prop changes, this component re-renders.

    // Edit State
    const [isEditing, setIsEditing] = useState(false);
    const [editName, setEditName] = useState('');
    const [editInclude, setEditInclude] = useState('');
    const [editExclude, setEditExclude] = useState('');
    const [isSaving, setIsSaving] = useState(false);

    const currentSource = initialSource;
    const isLibraryPack = Boolean(selectedLibraryPackId);

    useEffect(() => {
        // Reset edit mode when switching sources
        setIsEditing(false);
    }, [sourceId]);

    // Sync edit state when entering edit mode or source changes
    useEffect(() => {
        if (currentSource && !isEditing) {
            setEditName(currentSource.name);
            setEditInclude(currentSource.include_patterns?.join(', ') || '');
            setEditExclude(currentSource.exclude_patterns?.join(', ') || '');
        }
    }, [currentSource, isEditing]);

    const handleSaveSource = async () => {
        if (!onUpdateSource || !currentSource) return;
        try {
            setIsSaving(true);
            const includes = editInclude.split(',').map(s => s.trim()).filter(Boolean);
            const excludes = editExclude.split(',').map(s => s.trim()).filter(Boolean);
            await onUpdateSource(currentSource.id, editName, includes, excludes);
            setIsEditing(false);
        } catch (err) {
            console.error('Failed to update source:', err);
        } finally {
            setIsSaving(false);
        }
    };

    if (!currentSource && !isLibraryPack) {
        return (
            <div className="flex items-center justify-center flex-1 h-full text-[var(--text-secondary)]">
                Loading Workspace...
            </div>
        );
    }

    const isIndexing = currentSource && indexingResourceId === currentSource.id;
    const showingEditor = Boolean(selectedNotePath || selectedMemoryPath || selectedLibraryPackId);

    return (
        // Root container for the right-hand panel.
        // Scroll internally to avoid double scrollbars with MainLayout.
        <div className="flex flex-col flex-1 min-h-0 overflow-y-auto h-full">
            {/* Header / Stats Strip */}
            {currentSource && (
                <div className="px-6 py-3 border-b border-[var(--border-color)] bg-[var(--bg-content)] flex items-center justify-between">
                    {/* ... existing header content ... */}
                    <div className="flex items-center gap-4 flex-1 mr-5">
                        <div className="text-2xl flex items-center justify-center">
                            {currentSource.resource_type === 'local' ? '📁' : currentSource.resource_type === 'git' ? '🔗' : '📄'}
                        </div>
                        <div className="flex-1">
                            <div className="flex items-center gap-2.5 min-h-[32px]">
                                {isEditing ? (
                                    <input
                                        type="text"
                                        value={editName}
                                        onChange={(e) => setEditName(e.target.value)}
                                        placeholder="Source Name"
                                        className="text-[1.1rem] font-semibold text-[var(--text-active)] bg-[var(--bg-app)] border border-[var(--border-color)] rounded px-2 py-1 outline-none w-[200px]"
                                        disabled={isSaving}
                                    />
                                ) : (
                                    <h2 className="text-[1.2rem] font-semibold m-0 text-[var(--text-active)]">
                                        {currentSource.name}
                                    </h2>
                                )}

                                {!isEditing && (
                                    <button
                                        onClick={() => setIsEditing(true)}
                                        className="btn-ghost p-1 flex items-center justify-center transition-opacity"
                                        title="Edit Source"
                                    >
                                        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                            <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"></path>
                                            <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"></path>
                                        </svg>
                                    </button>
                                )}

                                {/* Status Badge */}
                                {isIndexing && (
                                    <span className="text-[0.75rem] bg-[var(--accent)] text-white px-2 py-0.5 rounded-full font-semibold">
                                        Indexing... {indexingProgress}
                                    </span>
                                )}
                            </div>

                            <div className="text-[0.8rem] text-[var(--text-secondary)] font-mono mt-1">
                                {currentSource.path}
                            </div>

                            {/* Edit Mode Inputs */}
                            {isEditing ? (
                                <div className="mt-2 flex gap-3 items-center">
                                    <div className="flex items-center gap-1.5">
                                        <span className="text-[0.75rem] font-semibold text-[var(--text-secondary)]">IN:</span>
                                        <input
                                            type="text"
                                            value={editInclude}
                                            onChange={(e) => setEditInclude(e.target.value)}
                                            placeholder="*.rs, src/**/*.ts"
                                            className="text-[0.8rem] font-mono bg-[var(--bg-app)] border border-[var(--border-color)] rounded px-1.5 py-0.5 w-[180px] text-[var(--text-primary)]"
                                            disabled={isSaving}
                                        />
                                    </div>
                                    <div className="flex items-center gap-1.5">
                                        <span className="text-[0.75rem] font-semibold text-[var(--text-secondary)]">EX:</span>
                                        <input
                                            type="text"
                                            value={editExclude}
                                            onChange={(e) => setEditExclude(e.target.value)}
                                            placeholder="target/*"
                                            className="text-[0.8rem] font-mono bg-[var(--bg-app)] border border-[var(--border-color)] rounded px-1.5 py-0.5 w-[180px] text-[var(--text-primary)]"
                                            disabled={isSaving}
                                        />
                                    </div>
                                    <div className="flex gap-2 ml-2">
                                        <button onClick={handleSaveSource} disabled={isSaving} className="btn-primary px-2 py-0.5 border-none">Save</button>
                                        <button onClick={() => setIsEditing(false)} disabled={isSaving} className="btn-outline px-2 py-0.5">Cancel</button>
                                    </div>
                                </div>
                            ) : (
                                (currentSource.include_patterns?.length > 0 || currentSource.exclude_patterns?.length > 0) && (
                                    <div className="text-[0.75rem] text-[var(--text-secondary)] mt-1 flex gap-3 flex-wrap">
                                        {currentSource.include_patterns?.length > 0 && <span title={currentSource.include_patterns.join(', ')}><span className="font-semibold text-[var(--text-secondary)]">IN: </span><span className="font-mono">{currentSource.include_patterns.join(', ')}</span></span>}
                                        {currentSource.exclude_patterns?.length > 0 && <span title={currentSource.exclude_patterns.join(', ')}><span className="font-semibold text-[var(--text-secondary)]">EX: </span><span className="font-mono">{currentSource.exclude_patterns.join(', ')}</span></span>}
                                    </div>
                                )
                            )}
                        </div>
                    </div>

                    <div className="flex items-center gap-4">
                        {/* Stats Compact */}
                        <div className="flex gap-4 mr-4 items-center text-[0.8rem] text-[var(--text-secondary)]">
                            {currentSource.stats && (
                                <>
                                    <span>{currentSource.stats.file_count} files</span>
                                    <span>{currentSource.stats.chunk_count} chunks</span>
                                    <span>{formatSize(currentSource.stats.total_size_bytes)}</span>
                                </>
                            )}
                        </div>

                        {/* Actions */}
                        <button
                            className={`px-3 py-1.5 transition-all ${
                                isIndexing 
                                    ? 'btn-outline opacity-70 cursor-not-allowed' 
                                    : 'btn-primary'
                            }`}
                            onClick={(e) => {
                                // Shift+click for full reindex, normal click for incremental
                                const mode = e.shiftKey ? 'full' : 'incremental';
                                onIndexResource?.(currentSource, mode);
                            }}
                            disabled={isIndexing}
                            title="Update Index (Shift+Click for full reindex)"
                        >
                            {isIndexing ? 'Indexing...' : 'Update'}
                        </button>
                    </div>
                </div>
            )}

            {/* Main Content Area: Conditional Render */}
            {/* Right side handles its own vertical scroll; sidebar scrolls independently. */}
            <div
                className={`relative ${
                    showingEditor
                        ? 'flex-[0_0_auto] block overflow-visible p-6'
                        : 'flex-1 flex overflow-hidden min-h-0'
                }`}
            >
                {selectedNotePath ? (
                    <div
                        className={showingEditor ? 'block' : 'flex-1 flex flex-col min-h-0 overflow-hidden'}
                    >
                        <CM6Editor sourceId={sourceId} docPath={selectedNotePath} docType="notes" scrollMode="container" />
                    </div>
                ) : selectedMemoryPath ? (
                    <div
                        className={showingEditor ? 'block' : 'flex-1 flex flex-col min-h-0 overflow-hidden'}
                    >
                        <CM6Editor sourceId={sourceId} docPath={selectedMemoryPath} docType="memory" scrollMode="container" />
                    </div>
                ) : selectedLibraryPackId ? (
                    <div
                        className={showingEditor ? 'block' : 'flex-1 flex flex-col min-h-0 overflow-hidden'}
                    >
                        <CM6Editor sourceId="library" docPath={selectedLibraryPackId} docType="library" scrollMode="container" />
                    </div>
                ) : (
                    <div
                        className="flex-1 flex flex-col h-full min-h-0"
                    >
                        
                        <GraphView sourceId={sourceId} />
                    </div>
                )}
            </div>
        </div>
    );
};
