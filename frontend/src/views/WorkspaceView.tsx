import React, { useEffect, useState } from 'react';
import { type Resource } from '../api';
import { GraphView } from '../components/GraphView';
import { MarkdownEditor } from '../components/MarkdownEditor';

interface WorkspaceViewProps {
    sourceId: string;
    source?: Resource;
    onIndexComplete?: () => void;
    // New props for indexing
    onIndexResource?: (resource: Resource) => void;
    indexingResourceId?: string | null;
    indexingProgress?: string | null;
    onUpdateSource?: (id: string, name: string, include: string[], exclude: string[]) => Promise<void>;
    selectedNotePath?: string | null;
}

export const WorkspaceView: React.FC<WorkspaceViewProps> = ({
    sourceId,
    source: initialSource,
    onIndexResource,
    indexingResourceId,
    indexingProgress,
    onUpdateSource,
    selectedNotePath
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

    if (!currentSource) {
        return <div className="workspace-loading">Loading Workspace...</div>;
    }

    const isIndexing = indexingResourceId === currentSource.id;

    return (
        <div className="workspace-view" style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
            {/* Header / Stats Strip */}
            <div className="workspace-header" style={{
                padding: '12px 24px',
                borderBottom: '1px solid var(--border-color)',
                backgroundColor: 'var(--bg-content)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between'
            }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: '16px', flex: 1, marginRight: '20px' }}>
                    <div className="source-icon" style={{ fontSize: '1.5rem', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                        {currentSource.resource_type === 'local' ? '📁' : currentSource.resource_type === 'git' ? '🔗' : '📄'}
                    </div>
                    <div style={{ flex: 1 }}>
                        <div style={{ display: 'flex', alignItems: 'center', gap: '10px', minHeight: '32px' }}>
                            {isEditing ? (
                                <input
                                    type="text"
                                    value={editName}
                                    onChange={(e) => setEditName(e.target.value)}
                                    placeholder="Source Name"
                                    style={{
                                        fontSize: '1.1rem',
                                        fontWeight: '600',
                                        color: 'var(--text-active)',
                                        background: 'var(--bg-app)',
                                        border: '1px solid var(--border-color)',
                                        borderRadius: '4px',
                                        padding: '4px 8px',
                                        outline: 'none',
                                        width: '200px'
                                    }}
                                    disabled={isSaving}
                                />
                            ) : (
                                <h2 style={{ fontSize: '1.2rem', fontWeight: '600', margin: 0, color: 'var(--text-active)' }}>
                                    {currentSource.name}
                                </h2>
                            )}

                            {!isEditing && (
                                <button
                                    onClick={() => setIsEditing(true)}
                                    style={{
                                        background: 'none',
                                        border: 'none',
                                        cursor: 'pointer',
                                        color: 'var(--text-muted)',
                                        padding: '4px',
                                        display: 'flex',
                                        alignItems: 'center',
                                        justifyContent: 'center',
                                        opacity: 0.7,
                                        transition: 'opacity 0.2s',
                                    }}
                                    title="Edit Source"
                                    onMouseOver={e => e.currentTarget.style.opacity = '1'}
                                    onMouseOut={e => e.currentTarget.style.opacity = '0.7'}
                                >
                                    <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                        <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"></path>
                                        <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"></path>
                                    </svg>
                                </button>
                            )}

                            {/* Status Badge */}
                            {isIndexing && (
                                <span style={{
                                    fontSize: '0.75rem',
                                    background: 'var(--accent)',
                                    color: 'white',
                                    padding: '2px 8px',
                                    borderRadius: '12px',
                                    fontWeight: '600'
                                }}>
                                    Indexing... {indexingProgress}
                                </span>
                            )}
                        </div>

                        <div style={{ fontSize: '0.8rem', color: 'var(--text-muted)', fontFamily: 'monospace', marginTop: '4px' }}>
                            {currentSource.path}
                        </div>

                        {/* Edit Mode Inputs */}
                        {isEditing ? (
                            <div style={{ marginTop: '8px', display: 'flex', gap: '12px', alignItems: 'center' }}>
                                {/* ... existing edit inputs ... */}
                                <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                                    <span style={{ fontSize: '0.75rem', fontWeight: 600, color: 'var(--text-muted)' }}>IN:</span>
                                    <input
                                        type="text"
                                        value={editInclude}
                                        onChange={(e) => setEditInclude(e.target.value)}
                                        placeholder="*.rs, src/**/*.ts"
                                        style={{ fontSize: '0.8rem', fontFamily: 'monospace', background: 'var(--bg-app)', border: '1px solid var(--border-color)', borderRadius: '4px', padding: '2px 6px', width: '180px', color: 'var(--text-primary)' }}
                                        disabled={isSaving}
                                    />
                                </div>
                                <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                                    <span style={{ fontSize: '0.75rem', fontWeight: 600, color: 'var(--text-muted)' }}>EX:</span>
                                    <input
                                        type="text"
                                        value={editExclude}
                                        onChange={(e) => setEditExclude(e.target.value)}
                                        placeholder="target/*"
                                        style={{ fontSize: '0.8rem', fontFamily: 'monospace', background: 'var(--bg-app)', border: '1px solid var(--border-color)', borderRadius: '4px', padding: '2px 6px', width: '180px', color: 'var(--text-primary)' }}
                                        disabled={isSaving}
                                    />
                                </div>
                                <div style={{ display: 'flex', gap: '8px', marginLeft: '8px' }}>
                                    <button onClick={handleSaveSource} disabled={isSaving} style={{ padding: '2px 8px', float: 'right', fontSize: '0.75rem', background: 'var(--accent)', color: 'white', border: 'none', borderRadius: '4px', cursor: 'pointer' }}>Save</button>
                                    <button onClick={() => setIsEditing(false)} disabled={isSaving} style={{ padding: '2px 8px', fontSize: '0.75rem', background: 'transparent', color: 'var(--text-secondary)', border: '1px solid var(--border-color)', borderRadius: '4px', cursor: 'pointer' }}>Cancel</button>
                                </div>
                            </div>
                        ) : (
                            (currentSource.include_patterns?.length > 0 || currentSource.exclude_patterns?.length > 0) && (
                                <div style={{ fontSize: '0.75rem', color: 'var(--text-secondary)', marginTop: '4px', display: 'flex', gap: '12px', flexWrap: 'wrap' }}>
                                    {currentSource.include_patterns?.length > 0 && <span title={currentSource.include_patterns.join(', ')}><span style={{ fontWeight: 600, color: 'var(--text-muted)' }}>IN: </span><span style={{ fontFamily: 'monospace' }}>{currentSource.include_patterns.join(', ')}</span></span>}
                                    {currentSource.exclude_patterns?.length > 0 && <span title={currentSource.exclude_patterns.join(', ')}><span style={{ fontWeight: 600, color: 'var(--text-muted)' }}>EX: </span><span style={{ fontFamily: 'monospace' }}>{currentSource.exclude_patterns.join(', ')}</span></span>}
                                </div>
                            )
                        )}
                    </div>
                </div>

                <div style={{ display: 'flex', alignItems: 'center', gap: '16px' }}>
                    {/* Stats Compact */}
                    <div style={{ display: 'flex', gap: '16px', marginRight: '16px', alignItems: 'center', fontSize: '0.8rem', color: 'var(--text-muted)' }}>
                        {currentSource.stats && (
                            <>
                                <span>{currentSource.stats.file_count} files</span>
                                <span>{currentSource.stats.chunk_count} chunks</span>
                            </>
                        )}
                    </div>

                    {/* Actions */}
                    <button
                        className="btn-action"
                        onClick={() => onIndexResource?.(currentSource)}
                        disabled={isIndexing}
                        style={{
                            padding: '6px 12px',
                            background: isIndexing ? 'var(--bg-sidebar)' : 'var(--accent)',
                            color: 'white',
                            border: isIndexing ? '1px solid var(--border-color)' : '1px solid var(--accent)',
                            opacity: isIndexing ? 0.7 : 1,
                            fontSize: '0.85rem'
                        }}
                    >
                        {isIndexing ? 'Indexing...' : 'Update Index'}
                    </button>
                </div>
            </div>

            {/* Main Content Area: Conditional Render */}
            <div className="workspace-body" style={{ flex: 1, display: 'flex', overflow: 'hidden', position: 'relative' }}>
                {selectedNotePath ? (
                    <div className="workspace-editor" style={{ flex: 1, display: 'flex', flexDirection: 'column', height: '100%' }}>
                        <MarkdownEditor sourceId={sourceId} notePath={selectedNotePath} />
                    </div>
                ) : (
                    <div className="workspace-graph" style={{ flex: 1, display: 'flex', flexDirection: 'column', height: '100%' }}>
                        <div style={{
                            padding: '8px 12px',
                            borderBottom: '1px solid var(--border-color)',
                            fontSize: '0.85rem',
                            fontWeight: 600,
                            color: 'var(--text-secondary)',
                            background: 'var(--bg-sidebar)',
                            display: 'flex',
                            justifyContent: 'space-between',
                            alignItems: 'center'
                        }}>
                            <span>GRAPH VIEW</span>
                        </div>
                        <GraphView sourceId={sourceId} />
                    </div>
                )}
            </div>
        </div>
    );
};
