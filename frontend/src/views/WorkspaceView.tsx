import React, { useEffect, useState } from 'react';
import { getProfile, updateProfile, generateProfile, type SourceProfile as SourceProfileType, type Resource } from '../api';
import { GraphView } from '../components/GraphView';

// For now, let's keep it simple and focused on display.
// This view assumes the Source is already selected.

interface WorkspaceViewProps {
    sourceId: string;
    source?: Resource;
    onIndexComplete?: () => void;
    // New props for indexing
    onIndexResource?: (resource: Resource) => void;
    indexingResourceId?: string | null;
    indexingProgress?: string | null;
    onUpdateSource?: (id: string, name: string, include: string[], exclude: string[]) => Promise<void>;
}

export const WorkspaceView: React.FC<WorkspaceViewProps> = ({
    sourceId,
    source: initialSource,
    onIndexResource,
    indexingResourceId,
    indexingProgress,
    onUpdateSource
}) => {
    // We don't need local state for source if it's passed as prop. 
    // If we were fetching it ourselves, we might need it, but typically we rely on the prop.
    // If the prop changes, this component re-renders.

    const [profile, setProfile] = useState<SourceProfileType | null>(null);
    const [loading, setLoading] = useState(false);
    const [generating, setGenerating] = useState(false);

    // Edit State
    const [isEditing, setIsEditing] = useState(false);
    const [editName, setEditName] = useState('');
    const [editInclude, setEditInclude] = useState('');
    const [editExclude, setEditExclude] = useState('');
    const [isSaving, setIsSaving] = useState(false);

    const currentSource = initialSource;

    useEffect(() => {
        loadProfile();
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

    const loadProfile = async () => {
        try {
            setLoading(true);
            const profileData = await getProfile(sourceId);
            setProfile(profileData);
        } catch (err) {
            console.error('Failed to load profile:', err);
        } finally {
            setLoading(false);
        }
    };

    const handleSaveProfile = async () => {
        if (!profile) return;
        try {
            setLoading(true);
            await updateProfile(sourceId, profile);
        } catch (err) {
            console.error('Failed to save profile:', err);
        } finally {
            setLoading(false);
        }
    };

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
            // Optionally show error state/toast here
        } finally {
            setIsSaving(false);
        }
    };

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const handleGenerate = async (_e: any) => {
        try {
            setGenerating(true);
            // Pass empty object to let backend decide/use defaults, or existing patterns
            const newProfile = await generateProfile(sourceId, {});
            setProfile(newProfile);
        } catch (err) {
            console.error('Failed to generate profile:', err);
        } finally {
            setGenerating(false);
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

                        {isEditing ? (
                            <div style={{ marginTop: '8px', display: 'flex', gap: '12px', alignItems: 'center' }}>
                                <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                                    <span style={{ fontSize: '0.75rem', fontWeight: 600, color: 'var(--text-muted)' }}>IN:</span>
                                    <input
                                        type="text"
                                        value={editInclude}
                                        onChange={(e) => setEditInclude(e.target.value)}
                                        placeholder="*.rs, src/**/*.ts"
                                        style={{
                                            fontSize: '0.8rem',
                                            fontFamily: 'monospace',
                                            background: 'var(--bg-app)',
                                            border: '1px solid var(--border-color)',
                                            borderRadius: '4px',
                                            padding: '2px 6px',
                                            width: '180px',
                                            color: 'var(--text-primary)'
                                        }}
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
                                        style={{
                                            fontSize: '0.8rem',
                                            fontFamily: 'monospace',
                                            background: 'var(--bg-app)',
                                            border: '1px solid var(--border-color)',
                                            borderRadius: '4px',
                                            padding: '2px 6px',
                                            width: '180px',
                                            color: 'var(--text-primary)'
                                        }}
                                        disabled={isSaving}
                                    />
                                </div>
                                <div style={{ display: 'flex', gap: '8px', marginLeft: '8px' }}>
                                    <button
                                        onClick={handleSaveSource}
                                        disabled={isSaving}
                                        style={{
                                            padding: '2px 8px',
                                            fontSize: '0.75rem',
                                            background: 'var(--accent)',
                                            color: 'white',
                                            border: 'none',
                                            borderRadius: '4px',
                                            cursor: 'pointer'
                                        }}
                                    >
                                        Save
                                    </button>
                                    <button
                                        onClick={() => setIsEditing(false)}
                                        disabled={isSaving}
                                        style={{
                                            padding: '2px 8px',
                                            fontSize: '0.75rem',
                                            background: 'transparent',
                                            color: 'var(--text-secondary)',
                                            border: '1px solid var(--border-color)',
                                            borderRadius: '4px',
                                            cursor: 'pointer'
                                        }}
                                    >
                                        Cancel
                                    </button>
                                </div>
                            </div>
                        ) : (
                            (currentSource.include_patterns?.length > 0 || currentSource.exclude_patterns?.length > 0) && (
                                <div style={{ fontSize: '0.75rem', color: 'var(--text-secondary)', marginTop: '4px', display: 'flex', gap: '12px', flexWrap: 'wrap' }}>
                                    {currentSource.include_patterns?.length > 0 && (
                                        <span title={currentSource.include_patterns.join(', ')}>
                                            <span style={{ fontWeight: 600, color: 'var(--text-muted)' }}>IN: </span>
                                            <span style={{ fontFamily: 'monospace' }}>
                                                {currentSource.include_patterns.join(', ')}
                                            </span>
                                        </span>
                                    )}
                                    {currentSource.exclude_patterns?.length > 0 && (
                                        <span title={currentSource.exclude_patterns.join(', ')}>
                                            <span style={{ fontWeight: 600, color: 'var(--text-muted)' }}>EX: </span>
                                            <span style={{ fontFamily: 'monospace' }}>
                                                {currentSource.exclude_patterns.join(', ')}
                                            </span>
                                        </span>
                                    )}
                                </div>
                            )
                        )}
                    </div>
                </div>

                <div style={{ display: 'flex', alignItems: 'center', gap: '24px' }}>
                    {/* Stats */}
                    <div style={{ display: 'flex', gap: '24px', marginRight: '16px', alignItems: 'center' }}>
                        {currentSource.latest_job?.finished_at && (
                            <div className="stat-item" style={{ borderRight: '1px solid var(--border-color)', paddingRight: '24px', marginRight: '8px' }}>
                                <span className="stat-value" style={{ fontSize: '0.9rem' }}>
                                    {new Date(currentSource.latest_job.finished_at).toLocaleString(undefined, {
                                        month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit'
                                    })}
                                </span>
                                <span className="stat-label">LAST INDEXED</span>
                            </div>
                        )}

                        {currentSource.stats && (
                            <>
                                <div className="stat-item">
                                    <span className="stat-value">{currentSource.stats.file_count}</span>
                                    <span className="stat-label">FILES</span>
                                </div>
                                <div className="stat-item">
                                    <span className="stat-value">{currentSource.stats.chunk_count}</span>
                                    <span className="stat-label">CHUNKS</span>
                                </div>
                            </>
                        )}
                    </div>

                    {/* Actions */}
                    <button
                        className="btn-action"
                        onClick={() => onIndexResource?.(currentSource)}
                        disabled={isIndexing}
                        style={{
                            padding: '6px 16px',
                            background: isIndexing ? 'var(--bg-sidebar)' : 'var(--accent)',
                            color: 'white',
                            border: isIndexing ? '1px solid var(--border-color)' : '1px solid var(--accent)',
                            opacity: isIndexing ? 0.7 : 1
                        }}
                    >
                        {isIndexing ? 'Indexing...' : 'Update Index'}
                    </button>
                </div>
            </div>

            {/* Main Content Area: Graph + Inspector */}
            <div className="workspace-body" style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>
                {/* Graph View (Main) */}
                <div className="workspace-main" style={{ flex: 1, position: 'relative' }}>
                    <GraphView sourceId={sourceId} />
                </div>

                {/* Inspector Panel (Right) - Fixed width for now */}
                <div className="workspace-inspector" style={{
                    width: '350px',
                    borderLeft: '1px solid var(--border-color)',
                    backgroundColor: 'var(--bg-sidebar)',
                    display: 'flex',
                    flexDirection: 'column'
                }}>
                    <div className="inspector-header" style={{
                        padding: '12px 16px',
                        borderBottom: '1px solid var(--border-color)',
                        fontWeight: '600',
                        fontSize: '0.85rem',
                        color: 'var(--text-secondary)',
                        textTransform: 'uppercase',
                        letterSpacing: '0.05em'
                    }}>
                        Project Profile
                    </div>

                    <div className="inspector-content" style={{ padding: '16px', flex: 1, overflowY: 'auto' }}>
                        {/* Profile Editor */}
                        <div className="form-group">
                            <label style={{ display: 'block', marginBottom: '8px', fontSize: '0.85rem', color: 'var(--text-secondary)' }}>
                                Description / Architecture
                            </label>
                            {profile ? (
                                <textarea
                                    value={profile.description}
                                    onChange={e => setProfile(prev => prev ? ({ ...prev, description: e.target.value }) : null)}
                                    rows={15}
                                    placeholder="Describe your project's tech stack, architecture, coding conventions..."
                                    style={{
                                        width: '100%',
                                        padding: '12px',
                                        borderRadius: '6px',
                                        border: '1px solid var(--border-color)',
                                        background: 'var(--bg-content)',
                                        color: 'var(--text-primary)',
                                        fontSize: '0.9rem',
                                        lineHeight: '1.5',
                                        resize: 'vertical',
                                        minHeight: '200px',
                                        fontFamily: 'inherit',
                                    }}
                                />
                            ) : (
                                <div style={{ color: 'var(--text-muted)', fontStyle: 'italic' }}>
                                    {loading ? 'Loading profile...' : 'No profile loaded.'}
                                </div>
                            )}
                        </div>

                        {/* Files List (for uploads type) */}
                        {currentSource.resource_type === 'uploads' && (
                            <div className="form-group" style={{ marginTop: '24px' }}>
                                <label style={{ display: 'block', marginBottom: '8px', fontSize: '0.85rem', color: 'var(--text-secondary)' }}>
                                    Uploaded Files
                                </label>
                                <div style={{
                                    background: 'var(--bg-content)',
                                    border: '1px solid var(--border-color)',
                                    borderRadius: '6px',
                                    maxHeight: '200px',
                                    overflowY: 'auto'
                                }}>
                                    {/* Placeholder for file list - in real implementation we'd fetch this via listUploadedFiles */}
                                    <div style={{ padding: '8px 12px', fontSize: '0.8rem', color: 'var(--text-muted)' }}>
                                        {currentSource.stats?.file_count
                                            ? `${currentSource.stats.file_count} files uploaded.`
                                            : 'No files uploaded yet.'}
                                    </div>
                                    {/* Link to full manager if needed */}
                                    <div style={{ padding: '8px 12px', borderTop: '1px solid var(--border-color)' }}>
                                        <button
                                            className="btn-secondary"
                                            style={{ width: '100%', fontSize: '0.8rem' }}
                                            onClick={() => {/* TODO: Implement upload/manage modal */ }}
                                        >
                                            Manage / Upload Files
                                        </button>
                                    </div>
                                </div>
                            </div>
                        )}
                    </div>

                    {/* Actions Footer */}
                    <div className="inspector-footer" style={{
                        padding: '16px',
                        borderTop: '1px solid var(--border-color)',
                        display: 'flex',
                        justifyContent: 'space-between',
                        gap: '12px'
                    }}>
                        <button
                            onClick={handleGenerate}
                            disabled={generating || loading || !profile}
                            className="btn-secondary"
                            style={{ flex: 1 }}
                        >
                            {generating ? 'Start...' : '✨ Auto-Generate'}
                        </button>
                        <button
                            onClick={handleSaveProfile}
                            disabled={loading || !profile}
                            className="btn-action btn-index"
                            style={{ flex: 1 }}
                        >
                            {loading ? 'Saving...' : 'Save Profile'}
                        </button>
                    </div>
                </div>
            </div>
        </div>
    );
};
