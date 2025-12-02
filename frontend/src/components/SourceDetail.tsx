import React, { useEffect, useState, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getProfile, updateProfile, generateProfile, listUploadedFiles, deleteUploadedFile, uploadFileWithProgress, renameResource, indexSource, updateResourcePatterns, getAppSettings, type SourceProfile as SourceProfileType, type Resource, type FileInfo, type UploadProgressInfo } from '../api';

interface SourceDetailProps {
    sourceId: string;
    onBack?: () => void;
    onIndexComplete?: () => void;
}

export const SourceDetail: React.FC<SourceDetailProps> = ({ sourceId, onBack, onIndexComplete }) => {
    const [profile, setProfile] = useState<SourceProfileType>({
        profile_name: '',
        description: '',
        tech_stack: [],
        architecture_notes: [],
        key_conventions: []
    });
    const [source, setSource] = useState<Resource | null>(null);
    const [loading, setLoading] = useState(false);
    const [generating, setGenerating] = useState(false);
    const [indexing, setIndexing] = useState(false);
    const [message, setMessage] = useState<{ text: string; type: 'success' | 'error' } | null>(null);
    const [docPatterns, setDocPatterns] = useState<string>('README*,');
    const [llmEnabled, setLlmEnabled] = useState(false);

    // Editing states
    const [isEditingName, setIsEditingName] = useState(false);
    const [editedName, setEditedName] = useState('');
    
    // Pattern editing states
    const [isEditingPatterns, setIsEditingPatterns] = useState(false);
    const [editedIncludePatterns, setEditedIncludePatterns] = useState('');
    const [editedExcludePatterns, setEditedExcludePatterns] = useState('');
    const [savingPatterns, setSavingPatterns] = useState(false);

    // File list state for uploads type
    const [files, setFiles] = useState<FileInfo[]>([]);
    const [uploading, setUploading] = useState(false);
    const [uploadProgress, setUploadProgress] = useState<number>(0);
    const [uploadPhase, setUploadPhase] = useState<string>('uploading');
    const [uploadStatusMessage, setUploadStatusMessage] = useState<string>('');
    const [deletingFile, setDeletingFile] = useState<string | null>(null);
    const [fileToDelete, setFileToDelete] = useState<string | null>(null);
    const [isDragging, setIsDragging] = useState(false);
    const fileInputRef = useRef<HTMLInputElement>(null);
    const dropZoneRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        loadData();
    }, []);

    // Listen for Tauri file drop events (for files dragged from Finder)
    useEffect(() => {
        if (source?.resource_type !== 'uploads') return;

        const unlisten = listen<{ paths: string[] }>('tauri://drag-drop', async (event) => {
            const paths = event.payload.paths;
            if (!paths || paths.length === 0) return;

            // For now, show a message - Tauri gives us file paths, not File objects
            // We'd need to read the files via Tauri's fs API
            console.log('Files dropped from Finder:', paths);
            setMessage({ text: `Dropped ${paths.length} file(s). Use "Click to browse" for now - Finder drag-drop coming soon.`, type: 'error' });
        });

        return () => {
            unlisten.then(fn => fn());
        };
    }, [source?.resource_type]);

    const loadData = async () => {
        try {
            setLoading(true);

            const [profileData, sourcesResponse, appSettings] = await Promise.all([
                getProfile(sourceId),
                fetch(`http://localhost:8787/api/resources`).then(r => r.json()),
                getAppSettings().catch(() => ({ llm_enabled: false }))
            ]);

            setProfile(profileData);
            setLlmEnabled(appSettings.llm_enabled);
            
            const foundSource = sourcesResponse.resources.find((r: Resource) => r.id === sourceId);
            if (foundSource) {
                setSource(foundSource);
                setEditedName(foundSource.name);
                setEditedIncludePatterns(foundSource.include_patterns.join(', '));
                setEditedExcludePatterns(foundSource.exclude_patterns.join(', '));
                if (foundSource.resource_type === 'uploads') {
                    await loadFiles();
                }
            }
        } catch (err) {
            console.error(err);
            setMessage({ text: 'Failed to load profile', type: 'error' });
        } finally {
            setLoading(false);
        }
    };

    const loadFiles = async () => {
        try {
            console.log('loadFiles: Fetching files for source', sourceId);
            const response = await listUploadedFiles(sourceId);
            console.log('loadFiles: Received', response.files.length, 'files:', response.files);
            setFiles(response.files);
        } catch (err) {
            console.error('Failed to load files:', err);
        }
    };

    const handleRename = async () => {
        if (!editedName.trim() || editedName === source?.name) {
            setIsEditingName(false);
            return;
        }

        try {
            await renameResource(sourceId, editedName.trim());
            setSource(prev => prev ? { ...prev, name: editedName.trim() } : null);
            setMessage({ text: '✓ Source renamed successfully', type: 'success' });
            setIsEditingName(false);
        } catch (err) {
            console.error(err);
            setMessage({ text: '✗ Failed to rename source', type: 'error' });
        }
    };

    const handleReindex = async () => {
        try {
            setIndexing(true);
            setMessage({ text: '⏳ Re-indexing source...', type: 'success' });
            await indexSource(sourceId);
            setMessage({ text: '✓ Indexing started! Check Activity tab for progress.', type: 'success' });
            onIndexComplete?.();
            setTimeout(() => loadData(), 2000);
        } catch (err) {
            console.error(err);
            setMessage({ text: '✗ Failed to start indexing', type: 'error' });
        } finally {
            setIndexing(false);
        }
    };

    const parsePatterns = (input: string): string[] => {
        return input
            .split(',')
            .map(p => p.trim())
            .filter(p => p.length > 0);
    };

    const handleSavePatterns = async () => {
        try {
            setSavingPatterns(true);
            const includePatterns = parsePatterns(editedIncludePatterns);
            const excludePatterns = parsePatterns(editedExcludePatterns);
            
            await updateResourcePatterns(sourceId, includePatterns, excludePatterns);
            setSource(prev => prev ? { 
                ...prev, 
                include_patterns: includePatterns,
                exclude_patterns: excludePatterns 
            } : null);
            setMessage({ text: '✓ Patterns updated. Re-index to apply changes.', type: 'success' });
            setIsEditingPatterns(false);
        } catch (err) {
            console.error(err);
            setMessage({ text: '✗ Failed to update patterns', type: 'error' });
        } finally {
            setSavingPatterns(false);
        }
    };

    const handleCancelPatternEdit = () => {
        if (source) {
            setEditedIncludePatterns(source.include_patterns.join(', '));
            setEditedExcludePatterns(source.exclude_patterns.join(', '));
        }
        setIsEditingPatterns(false);
    };

    const handleUploadClick = () => {
        fileInputRef.current?.click();
    };

    const handleFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
        const selectedFiles = e.target.files;
        if (!selectedFiles || selectedFiles.length === 0) return;
        await processFiles(Array.from(selectedFiles));
    };

    const handleDeleteFile = (filename: string) => {
        setFileToDelete(filename);
    };

    const confirmDeleteFile = async () => {
        if (!fileToDelete) return;

        setDeletingFile(fileToDelete);
        setFileToDelete(null);
        setMessage(null);

        try {
            const result = await deleteUploadedFile(sourceId, fileToDelete);
            setMessage({ text: `✓ Deleted "${result.filename}": ${result.chunks_deleted} chunks removed`, type: 'success' });
            await loadFiles();
            await loadData();
        } catch (err) {
            setMessage({ text: `✗ Failed to delete: ${err}`, type: 'error' });
        } finally {
            setDeletingFile(null);
        }
    };

    const handleDragEnter = (e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        setIsDragging(true);
    };

    const handleDragLeave = (e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        if (dropZoneRef.current && !dropZoneRef.current.contains(e.relatedTarget as Node)) {
            setIsDragging(false);
        }
    };

    const handleDragOver = (e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
    };

    const handleDrop = async (e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        setIsDragging(false);

        const droppedFiles = e.dataTransfer.files;
        if (!droppedFiles || droppedFiles.length === 0) return;

        await processFiles(Array.from(droppedFiles));
    };

    const processFiles = async (filesToUpload: File[]) => {
        setUploading(true);
        setUploadProgress(0);
        setUploadPhase('uploading');
        setUploadStatusMessage('Starting upload...');
        setMessage(null);

        try {
            let uploadedCount = 0;
            for (const file of filesToUpload) {
                console.log('processFiles: Starting upload for', file.name);
                setUploadProgress(0);
                setUploadPhase('uploading');
                setUploadStatusMessage(`Uploading ${file.name}...`);
                
                const result = await uploadFileWithProgress(sourceId, file, (info: UploadProgressInfo) => {
                    setUploadProgress(info.progress);
                    setUploadPhase(info.phase);
                    setUploadStatusMessage(info.message);
                });
                
                console.log('processFiles: Upload complete for', file.name, 'result:', result);
                uploadedCount++;
                setMessage({ text: `✓ Uploaded ${uploadedCount}/${filesToUpload.length}: "${result.filename}" (${result.chunks_created} chunks)`, type: 'success' });
            }
            console.log('processFiles: All uploads complete, waiting for backend to sync...');
            // Small delay to ensure LanceDB has committed the data
            await new Promise(resolve => setTimeout(resolve, 500));
            console.log('processFiles: Refreshing file list...');
            await loadFiles();
            console.log('processFiles: File list refreshed');
            await loadData();
        } catch (err) {
            setMessage({ text: `✗ Failed to upload: ${err}`, type: 'error' });
        } finally {
            setUploading(false);
            setUploadProgress(0);
            setUploadPhase('uploading');
            setUploadStatusMessage('');
            if (fileInputRef.current) {
                fileInputRef.current.value = '';
            }
        }
    };

    const handleSave = async () => {
        try {
            setLoading(true);
            await updateProfile(sourceId, profile);
            setMessage({ text: '✓ Profile saved successfully', type: 'success' });
        } catch (err) {
            console.error(err);
            setMessage({ text: '✗ Failed to save profile', type: 'error' });
        } finally {
            setLoading(false);
        }
    };

    const handleGenerate = async () => {
        try {
            setGenerating(true);
            setMessage({ text: '⏳ Generating profile from source files... This may take a moment.', type: 'success' });
            const files = docPatterns
                .split(',')
                .map(p => p.trim())
                .filter(p => p.length > 0);

            const data = await generateProfile(sourceId, { files: files.length > 0 ? files : undefined });
            setProfile(data);
            setMessage({ text: '✓ Profile generated successfully!', type: 'success' });
        } catch (err) {
            console.error(err);
            setMessage({ text: '✗ Failed to generate profile', type: 'error' });
        } finally {
            setGenerating(false);
        }
    };

    const formatBytes = (bytes: number) => {
        if (bytes < 1024) return `${bytes} B`;
        if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
        return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    };

    const styles = {
        card: {
            background: 'var(--surface)',
            borderRadius: '12px',
            border: '1px solid var(--border)',
            padding: '1.25rem',
            marginBottom: '1rem',
        } as React.CSSProperties,
        statBox: {
            textAlign: 'center' as const,
            padding: '0.5rem 1rem',
        },
        statValue: {
            fontWeight: '700',
            color: 'var(--primary)',
            fontSize: '1.75rem',
            lineHeight: '1.2',
        },
        statLabel: {
            color: 'var(--text-muted)',
            fontSize: '0.75rem',
            textTransform: 'uppercase' as const,
            letterSpacing: '0.05em',
            marginTop: '0.25rem',
        },
        sectionTitle: {
            fontSize: '0.7rem',
            fontWeight: '600',
            color: 'var(--text-muted)',
            textTransform: 'uppercase' as const,
            letterSpacing: '0.08em',
            marginBottom: '0.5rem',
        },
        patternTag: {
            display: 'inline-block',
            padding: '0.25rem 0.5rem',
            borderRadius: '4px',
            fontFamily: 'monospace',
            fontSize: '0.8rem',
            marginRight: '0.5rem',
            marginBottom: '0.25rem',
        },
        btn: {
            padding: '0.5rem 0.875rem',
            background: 'transparent',
            border: '1px solid var(--border)',
            borderRadius: '6px',
            cursor: 'pointer',
            fontSize: '0.8rem',
            color: 'var(--text)',
            transition: 'all 0.15s ease',
            whiteSpace: 'nowrap' as const,
            display: 'inline-flex',
            alignItems: 'center',
            justifyContent: 'center',
            gap: '0.35rem',
            flex: 'none',
            width: 'fit-content',
        } as React.CSSProperties,
        btnPrimary: {
            padding: '0.5rem 0.875rem',
            background: 'var(--primary)',
            color: 'white',
            border: 'none',
            borderRadius: '6px',
            cursor: 'pointer',
            fontSize: '0.8rem',
            fontWeight: '500',
            transition: 'all 0.15s ease',
            whiteSpace: 'nowrap' as const,
            display: 'inline-flex',
            alignItems: 'center',
            justifyContent: 'center',
            gap: '0.35rem',
            flex: 'none',
            width: 'fit-content',
        } as React.CSSProperties,
    };

    return (
        <div className="view" style={{ width: '100%', maxWidth: '900px', margin: '0 auto' }}>
            {/* Navigation */}
            {onBack && (
                <div style={{ marginBottom: '1rem' }}>
                    <button onClick={onBack} style={styles.btn}>
                        ← Back to Sources
                    </button>
                </div>
            )}

            {/* Toast Message */}
            {message && (
                <div
                    style={{
                        padding: '0.75rem 1rem',
                        borderRadius: '8px',
                        marginBottom: '1rem',
                        fontSize: '0.875rem',
                        background: message.type === 'success' ? 'rgba(34, 197, 94, 0.1)' : 'rgba(239, 68, 68, 0.1)',
                        color: message.type === 'success' ? '#22c55e' : '#ef4444',
                        border: `1px solid ${message.type === 'success' ? 'rgba(34, 197, 94, 0.2)' : 'rgba(239, 68, 68, 0.2)'}`,
                    }}
                >
                    {message.text}
                </div>
            )}

            {/* Source Header Card */}
            {source && (
                <div style={styles.card}>
                    <div style={{ display: 'flex', alignItems: 'flex-start', gap: '1rem' }}>
                        {/* Icon */}
                        <div style={{
                            fontSize: '2rem',
                            width: '56px',
                            height: '56px',
                            display: 'flex',
                            alignItems: 'center',
                            justifyContent: 'center',
                            background: 'rgba(100, 108, 255, 0.1)',
                            borderRadius: '12px',
                        }}>
                            {source.resource_type === 'local' ? '📁' : source.resource_type === 'git' ? '🔗' : source.resource_type === 'uploads' ? '📥' : '🌐'}
                        </div>

                        {/* Info */}
                        <div style={{ flex: 1, minWidth: 0 }}>
                            {isEditingName ? (
                                <div style={{ display: 'flex', alignItems: 'center', gap: '0.5rem', marginBottom: '0.5rem' }}>
                                    <input
                                        type="text"
                                        value={editedName}
                                        onChange={(e) => setEditedName(e.target.value)}
                                        onKeyDown={(e) => {
                                            if (e.key === 'Enter') handleRename();
                                            if (e.key === 'Escape') {
                                                setEditedName(source.name);
                                                setIsEditingName(false);
                                            }
                                        }}
                                        autoFocus
                                        style={{
                                            fontSize: '1.25rem',
                                            fontWeight: '600',
                                            padding: '0.35rem 0.75rem',
                                            borderRadius: '6px',
                                            border: '2px solid var(--primary)',
                                            background: 'var(--background)',
                                            color: 'var(--text)',
                                            width: '280px',
                                            outline: 'none',
                                        }}
                                    />
                                    <button onClick={handleRename} style={{ ...styles.btnPrimary, padding: '0.4rem 0.75rem' }}>
                                        Save
                                    </button>
                                    <button
                                        onClick={() => { setEditedName(source.name); setIsEditingName(false); }}
                                        style={styles.btn}
                                    >
                                        Cancel
                                    </button>
                                </div>
                            ) : (
                                <div style={{ display: 'flex', alignItems: 'center', gap: '0.5rem', marginBottom: '0.5rem' }}>
                                    <h2 style={{ margin: 0, fontSize: '1.35rem', fontWeight: '600' }}>{source.name}</h2>
                                    <button
                                        onClick={() => setIsEditingName(true)}
                                        style={{ ...styles.btn, padding: '0.3rem 0.6rem', fontSize: '0.75rem' }}
                                    >
                                        ✏️ Rename
                                    </button>
                                </div>
                            )}
                            <div style={{ display: 'flex', alignItems: 'center', gap: '0.75rem', flexWrap: 'wrap' }}>
                                <span style={{
                                    background: 'rgba(100, 108, 255, 0.15)',
                                    color: 'var(--primary)',
                                    padding: '0.15rem 0.5rem',
                                    borderRadius: '4px',
                                    fontSize: '0.7rem',
                                    fontWeight: '600',
                                    textTransform: 'uppercase',
                                    letterSpacing: '0.03em',
                                }}>
                                    {source.resource_type}
                                </span>
                                {source.resource_type !== 'uploads' && (
                                    <span style={{
                                        fontFamily: 'monospace',
                                        fontSize: '0.8rem',
                                        color: 'var(--text-muted)',
                                        overflow: 'hidden',
                                        textOverflow: 'ellipsis',
                                        whiteSpace: 'nowrap',
                                    }} title={source.path}>
                                        {source.path}
                                    </span>
                                )}
                            </div>
                        </div>

                        {/* Stats */}
                        {source.stats && (
                            <div style={{ display: 'flex', gap: '0.5rem', borderLeft: '1px solid var(--border)', paddingLeft: '1rem', marginLeft: '0.5rem' }}>
                                <div style={styles.statBox}>
                                    <div style={styles.statValue}>{source.stats.file_count}</div>
                                    <div style={styles.statLabel}>files</div>
                                </div>
                                <div style={styles.statBox}>
                                    <div style={styles.statValue}>{source.stats.chunk_count}</div>
                                    <div style={styles.statLabel}>chunks</div>
                                </div>
                                <div style={styles.statBox}>
                                    <div style={styles.statValue}>{formatBytes(source.stats.total_size_bytes)}</div>
                                    <div style={styles.statLabel}>size</div>
                                </div>
                            </div>
                        )}
                    </div>
                </div>
            )}

            {/* Patterns & Actions Card (for non-uploads) */}
            {source && source.resource_type !== 'uploads' && (
                <div style={styles.card}>
                    {isEditingPatterns ? (
                        /* Edit Mode */
                        <div>
                            <div style={{ display: 'flex', gap: '1.5rem', marginBottom: '1rem' }}>
                                <div style={{ flex: 1 }}>
                                    <label style={{ display: 'block', fontSize: '0.75rem', fontWeight: '600', color: 'var(--text-muted)', marginBottom: '0.5rem', textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                                        Include Patterns
                                    </label>
                                    <input
                                        type="text"
                                        value={editedIncludePatterns}
                                        onChange={(e) => setEditedIncludePatterns(e.target.value)}
                                        placeholder="*.cs, *.md, *.json"
                                        style={{
                                            width: '100%',
                                            padding: '0.5rem 0.75rem',
                                            borderRadius: '6px',
                                            border: '1px solid var(--border)',
                                            background: 'var(--background)',
                                            color: 'var(--text)',
                                            fontSize: '0.85rem',
                                            fontFamily: 'monospace',
                                        }}
                                    />
                                    <span style={{ fontSize: '0.7rem', color: 'var(--text-muted)', marginTop: '0.25rem', display: 'block' }}>
                                        Comma-separated glob patterns
                                    </span>
                                </div>
                                <div style={{ flex: 1 }}>
                                    <label style={{ display: 'block', fontSize: '0.75rem', fontWeight: '600', color: 'var(--text-muted)', marginBottom: '0.5rem', textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                                        Exclude Patterns
                                    </label>
                                    <input
                                        type="text"
                                        value={editedExcludePatterns}
                                        onChange={(e) => setEditedExcludePatterns(e.target.value)}
                                        placeholder="*.meta, *.asset, node_modules"
                                        style={{
                                            width: '100%',
                                            padding: '0.5rem 0.75rem',
                                            borderRadius: '6px',
                                            border: '1px solid var(--border)',
                                            background: 'var(--background)',
                                            color: 'var(--text)',
                                            fontSize: '0.85rem',
                                            fontFamily: 'monospace',
                                        }}
                                    />
                                    <span style={{ fontSize: '0.7rem', color: 'var(--text-muted)', marginTop: '0.25rem', display: 'block' }}>
                                        Files matching these will be skipped
                                    </span>
                                </div>
                            </div>
                            <div style={{ display: 'flex', gap: '0.5rem' }}>
                                <button
                                    onClick={handleSavePatterns}
                                    disabled={savingPatterns}
                                    style={{
                                        ...styles.btnPrimary,
                                        opacity: savingPatterns ? 0.6 : 1,
                                        cursor: savingPatterns ? 'wait' : 'pointer',
                                    }}
                                >
                                    {savingPatterns ? '⏳ Saving...' : '✓ Save Patterns'}
                                </button>
                                <button onClick={handleCancelPatternEdit} style={styles.btn}>
                                    ✕ Cancel
                                </button>
                            </div>
                        </div>
                    ) : (
                        /* View Mode */
                        <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: '1rem' }}>
                            <div style={{ flex: 1 }}>
                                <div style={{ display: 'flex', gap: '2rem', marginBottom: '0.5rem' }}>
                                    <div>
                                        <div style={styles.sectionTitle}>Include Patterns</div>
                                        <div>
                                            {source.include_patterns.length > 0 ? (
                                                source.include_patterns.map((p, i) => (
                                                    <span key={i} style={{ ...styles.patternTag, background: 'rgba(34, 197, 94, 0.15)', color: '#22c55e' }}>
                                                        {p}
                                                    </span>
                                                ))
                                            ) : (
                                                <span style={{ ...styles.patternTag, background: 'rgba(34, 197, 94, 0.1)', color: '#22c55e' }}>* (all files)</span>
                                            )}
                                        </div>
                                    </div>
                                    <div>
                                        <div style={styles.sectionTitle}>Exclude Patterns</div>
                                        <div>
                                            {source.exclude_patterns.length > 0 ? (
                                                source.exclude_patterns.map((p, i) => (
                                                    <span key={i} style={{ ...styles.patternTag, background: 'rgba(239, 68, 68, 0.15)', color: '#ef4444' }}>
                                                        {p}
                                                    </span>
                                                ))
                                            ) : (
                                                <span style={{ color: 'var(--text-muted)', fontSize: '0.85rem', fontStyle: 'italic' }}>None</span>
                                            )}
                                        </div>
                                    </div>
                                </div>
                                <p style={{ color: 'var(--text-muted)', fontSize: '0.75rem', margin: 0, fontStyle: 'italic' }}>
                                    Files in <code style={{ background: 'rgba(255,255,255,0.1)', padding: '0.1rem 0.3rem', borderRadius: '3px' }}>.gitignore</code> are always excluded automatically.
                                </p>
                            </div>
                            <div style={{ display: 'flex', gap: '0.5rem', flexShrink: 0 }}>
                                <button
                                    onClick={() => setIsEditingPatterns(true)}
                                    style={styles.btn}
                                    title="Edit patterns"
                                >
                                    ✏️ Edit
                                </button>
                                <button
                                    onClick={handleReindex}
                                    disabled={indexing}
                                    style={{
                                        ...styles.btn,
                                        opacity: indexing ? 0.6 : 1,
                                        cursor: indexing ? 'wait' : 'pointer',
                                    }}
                                >
                                    {indexing ? '⏳ Indexing...' : '🔄 Re-index'}
                                </button>
                            </div>
                        </div>
                    )}
                </div>
            )}

            {/* File Upload section for uploads sources */}
            {source?.resource_type === 'uploads' && (
                <div style={styles.card}>
                    <input
                        ref={fileInputRef}
                        type="file"
                        multiple
                        accept=".pdf,.docx,.doc,.txt,.md,.markdown,.json,.yaml,.yml,.toml,.csv,.xml,.html,.htm,.rst,.tex"
                        style={{ display: 'none' }}
                        onChange={handleFileChange}
                    />

                    <div
                        ref={dropZoneRef}
                        onDragEnter={handleDragEnter}
                        onDragLeave={handleDragLeave}
                        onDragOver={handleDragOver}
                        onDrop={handleDrop}
                        onClick={handleUploadClick}
                        style={{
                            border: `2px dashed ${isDragging ? 'var(--primary)' : 'var(--border)'}`,
                            borderRadius: '10px',
                            padding: '1.5rem',
                            textAlign: 'center',
                            cursor: uploading ? 'wait' : 'pointer',
                            background: isDragging ? 'rgba(100, 108, 255, 0.08)' : 'transparent',
                            transition: 'all 0.2s ease',
                            marginBottom: files.length > 0 ? '1rem' : 0,
                        }}
                    >
                        <div style={{ fontSize: '2rem', marginBottom: '0.5rem' }}>
                            {uploading ? '⏳' : isDragging ? '📥' : '📤'}
                        </div>
                        <div style={{ fontSize: '0.95rem', fontWeight: '500', color: 'var(--text)', marginBottom: '0.25rem' }}>
                            {uploading 
                                ? `${uploadStatusMessage} (${uploadProgress}%)`
                                : isDragging 
                                    ? 'Drop files here' 
                                    : 'Drop files here or click to browse'}
                        </div>
                        {uploading && (
                            <>
                                <div style={{ 
                                    width: '100%', 
                                    maxWidth: '300px', 
                                    height: '6px', 
                                    backgroundColor: 'var(--border)', 
                                    borderRadius: '3px', 
                                    overflow: 'hidden',
                                    margin: '0.5rem auto'
                                }}>
                                    <div style={{ 
                                        width: `${uploadProgress}%`, 
                                        height: '100%', 
                                        backgroundColor: 'var(--primary)', 
                                        transition: 'width 0.3s ease',
                                        borderRadius: '3px'
                                    }} />
                                </div>
                                <div style={{ fontSize: '0.75rem', color: 'var(--text-muted)', marginTop: '0.25rem' }}>
                                    {uploadPhase === 'embedding' ? '⚡ This may take a moment for large files' : ''}
                                </div>
                            </>
                        )}
                        <div style={{ fontSize: '0.8rem', color: 'var(--text-muted)' }}>
                            PDF, DOCX, TXT, MD, JSON, YAML, and more
                        </div>
                    </div>

                    {files.length > 0 && (
                        <>
                            <div style={{ marginBottom: '0.5rem' }}>
                                <span style={{ fontSize: '0.8rem', color: 'var(--text-muted)' }}>
                                    📁 {files.length} file{files.length !== 1 ? 's' : ''} uploaded
                                </span>
                            </div>
                            <div style={{ background: 'var(--background)', borderRadius: '8px', overflow: 'hidden' }}>
                                <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                                    <thead>
                                        <tr style={{ borderBottom: '1px solid var(--border)' }}>
                                            <th style={{ padding: '0.5rem 0.75rem', textAlign: 'left', fontWeight: '500', color: 'var(--text-muted)', fontSize: '0.75rem' }}>Filename</th>
                                            <th style={{ padding: '0.5rem 0.75rem', textAlign: 'right', fontWeight: '500', color: 'var(--text-muted)', fontSize: '0.75rem', width: '70px' }}>Chunks</th>
                                            <th style={{ padding: '0.5rem 0.75rem', textAlign: 'right', fontWeight: '500', color: 'var(--text-muted)', fontSize: '0.75rem', width: '50px' }}></th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {files.map((file) => (
                                            <tr key={file.filename} style={{ borderBottom: '1px solid var(--border)' }}>
                                                <td style={{ padding: '0.5rem 0.75rem' }}>
                                                    <span style={{ fontFamily: 'monospace', fontSize: '0.8rem' }}>{file.filename}</span>
                                                </td>
                                                <td style={{ padding: '0.5rem 0.75rem', textAlign: 'right', color: 'var(--text-muted)', fontSize: '0.8rem' }}>
                                                    {file.chunk_count}
                                                </td>
                                                <td style={{ padding: '0.5rem 0.75rem', textAlign: 'right' }}>
                                                    <button
                                                        onClick={(e) => { e.stopPropagation(); handleDeleteFile(file.filename); }}
                                                        disabled={deletingFile === file.filename}
                                                        style={{
                                                            background: 'transparent',
                                                            border: 'none',
                                                            color: deletingFile === file.filename ? 'var(--text-muted)' : '#ef4444',
                                                            cursor: deletingFile === file.filename ? 'wait' : 'pointer',
                                                            padding: '0.2rem 0.4rem',
                                                            borderRadius: '4px',
                                                            fontSize: '0.75rem',
                                                        }}
                                                        title="Delete file"
                                                    >
                                                        {deletingFile === file.filename ? '...' : '✕'}
                                                    </button>
                                                </td>
                                            </tr>
                                        ))}
                                    </tbody>
                                </table>
                            </div>
                        </>
                    )}
                </div>
            )}

            {/* Profile Generation Card (for non-uploads) */}
            {source?.resource_type !== 'uploads' && (
                <div style={styles.card}>
                    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: '1rem' }}>
                        <div>
                            <div style={styles.sectionTitle}>Profile Generator</div>
                            <p style={{ color: 'var(--text-muted)', fontSize: '0.8rem', margin: 0 }}>
                                {llmEnabled 
                                    ? 'Auto-generate from documentation files'
                                    : 'Enable Local LLM in Settings to use auto-generate'
                                }
                            </p>
                        </div>
                        <div style={{ display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
                            <input
                                type="text"
                                value={docPatterns}
                                onChange={e => setDocPatterns(e.target.value)}
                                placeholder="*.md, README*"
                                disabled={!llmEnabled}
                                style={{
                                    width: '140px',
                                    padding: '0.5rem 0.75rem',
                                    borderRadius: '6px',
                                    border: '1px solid var(--border)',
                                    background: 'var(--background)',
                                    color: 'var(--text)',
                                    fontSize: '0.8rem',
                                    fontFamily: 'monospace',
                                    opacity: llmEnabled ? 1 : 0.5,
                                }}
                            />
                            <button
                                onClick={handleGenerate}
                                disabled={generating || loading || !llmEnabled}
                                title={!llmEnabled ? 'Enable Local LLM in Settings' : ''}
                                style={{
                                    ...styles.btnPrimary,
                                    opacity: (generating || loading || !llmEnabled) ? 0.5 : 1,
                                    cursor: (generating || loading || !llmEnabled) ? 'not-allowed' : 'pointer',
                                }}
                            >
                                {generating ? '⏳ Generating...' : '✨ Generate'}
                            </button>
                        </div>
                    </div>
                </div>
            )}

            {/* Profile Editor Card (for non-uploads) */}
            {source?.resource_type !== 'uploads' && (
                <div style={styles.card}>
                    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '0.75rem' }}>
                        <div>
                            <div style={styles.sectionTitle}>Project Profile</div>
                            <p style={{ color: 'var(--text-muted)', fontSize: '0.8rem', margin: 0, whiteSpace: 'nowrap' }}>
                                Tech stack, architecture, and conventions
                            </p>
                        </div>
                        <button
                            onClick={handleSave}
                            disabled={loading}
                            style={{
                                ...styles.btnPrimary,
                                opacity: loading ? 0.6 : 1,
                                cursor: loading ? 'wait' : 'pointer',
                            }}
                        >
                            {loading ? '💾 Saving...' : '💾 Save'}
                        </button>
                    </div>
                    <textarea
                        value={profile.description}
                        onChange={e => setProfile(prev => ({ ...prev, description: e.target.value }))}
                        rows={10}
                        placeholder="Describe your project's tech stack, architecture, coding conventions..."
                        style={{
                            width: '100%',
                            padding: '0.75rem',
                            borderRadius: '8px',
                            border: '1px solid var(--border)',
                            background: 'var(--background)',
                            color: 'var(--text)',
                            fontSize: '0.9rem',
                            lineHeight: '1.6',
                            resize: 'vertical',
                            fontFamily: 'inherit',
                        }}
                    />
                </div>
            )}

            {/* Delete File Confirmation Modal */}
            {fileToDelete && (
                <div className="modal-overlay" style={{
                    position: 'fixed',
                    top: 0,
                    left: 0,
                    right: 0,
                    bottom: 0,
                    backgroundColor: 'rgba(0, 0, 0, 0.5)',
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    zIndex: 1000,
                }}>
                    <div className="modal-content" style={{
                        backgroundColor: 'var(--card-bg)',
                        borderRadius: '12px',
                        padding: '1.5rem',
                        maxWidth: '400px',
                        width: '90%',
                        border: '1px solid var(--border)',
                    }}>
                        <h3 style={{ marginTop: 0, marginBottom: '1rem', color: 'var(--text)' }}>Delete File?</h3>
                        <p style={{ color: 'var(--text-secondary)', marginBottom: '1.5rem' }}>
                            Are you sure you want to delete "<strong>{fileToDelete}</strong>" and all its chunks? This cannot be undone.
                        </p>
                        <div style={{ display: 'flex', gap: '0.75rem', justifyContent: 'flex-end' }}>
                            <button
                                onClick={() => setFileToDelete(null)}
                                style={{
                                    padding: '0.5rem 1rem',
                                    borderRadius: '6px',
                                    border: '1px solid var(--border)',
                                    background: 'transparent',
                                    color: 'var(--text)',
                                    cursor: 'pointer',
                                }}
                            >
                                Cancel
                            </button>
                            <button
                                onClick={confirmDeleteFile}
                                style={{
                                    padding: '0.5rem 1rem',
                                    borderRadius: '6px',
                                    border: 'none',
                                    background: '#ef4444',
                                    color: 'white',
                                    cursor: 'pointer',
                                }}
                            >
                                Delete
                            </button>
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
};
