import React, { useEffect, useState, useRef } from 'react';
import { getProfile, updateProfile, generateProfile, listUploadedFiles, deleteUploadedFile, uploadFile, type SourceProfile as SourceProfileType, type Resource, type FileInfo } from '../api';

export const SourceProfile: React.FC<{ sourceId: string; onBack?: () => void }> = ({ sourceId, onBack }) => {
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
    const [message, setMessage] = useState<{ text: string; type: 'success' | 'error' } | null>(null);
    const [docPatterns, setDocPatterns] = useState<string>('README*,');

    // File list state for uploads type
    const [files, setFiles] = useState<FileInfo[]>([]);
    const [uploading, setUploading] = useState(false);
    const [deletingFile, setDeletingFile] = useState<string | null>(null);
    const [isDragging, setIsDragging] = useState(false);
    const fileInputRef = useRef<HTMLInputElement>(null);
    const dropZoneRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        loadData();
    }, []);

    const loadData = async () => {
        try {
            setLoading(true);

            // Load both profile and source info
            const [profileData, sourcesResponse] = await Promise.all([
                getProfile(sourceId),
                fetch(`http://localhost:7000/api/resources`).then(r => r.json())
            ]);

            setProfile(profileData);
            const foundSource = sourcesResponse.resources.find((r: Resource) => r.id === sourceId);
            if (foundSource) {
                setSource(foundSource);
                // Load files for uploads sources
                if (foundSource.resource_type === 'uploads') {
                    await loadFiles();
                }
            }
        } catch (err) {
            // Only show error if it's not the default profile being loaded
            // Actually, if we get here it means the fetch failed or json parse failed
            console.error(err);
            setMessage({ text: 'Failed to load profile', type: 'error' });
        } finally {
            setLoading(false);
        }
    };

    const loadFiles = async () => {
        try {
            const response = await listUploadedFiles(sourceId);
            setFiles(response.files);
        } catch (err) {
            console.error('Failed to load files:', err);
        }
    };

    const handleUploadClick = () => {
        fileInputRef.current?.click();
    };

    const handleFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
        const selectedFiles = e.target.files;
        if (!selectedFiles || selectedFiles.length === 0) return;
        await processFiles(Array.from(selectedFiles));
    };

    const handleDeleteFile = async (filename: string) => {
        if (!confirm(`Delete "${filename}" and all its chunks?`)) return;

        setDeletingFile(filename);
        setMessage(null);

        try {
            const result = await deleteUploadedFile(sourceId, filename);
            setMessage({ text: `✓ Deleted "${result.filename}": ${result.chunks_deleted} chunks removed`, type: 'success' });
            await loadFiles();
            // Also reload source to get updated stats
            await loadData();
        } catch (err) {
            setMessage({ text: `✗ Failed to delete: ${err}`, type: 'error' });
        } finally {
            setDeletingFile(null);
        }
    };

    // Drag and drop handlers
    const handleDragEnter = (e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        setIsDragging(true);
    };

    const handleDragLeave = (e: React.DragEvent) => {
        e.preventDefault();
        e.stopPropagation();
        // Only set dragging to false if we're leaving the drop zone entirely
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
        setMessage(null);

        try {
            let uploadedCount = 0;
            for (const file of filesToUpload) {
                const result = await uploadFile(sourceId, file);
                uploadedCount++;
                setMessage({ text: `✓ Uploaded ${uploadedCount}/${filesToUpload.length}: "${result.filename}" (${result.chunks_created} chunks)`, type: 'success' });
            }
            await loadFiles();
            await loadData();
        } catch (err) {
            setMessage({ text: `✗ Failed to upload: ${err}`, type: 'error' });
        } finally {
            setUploading(false);
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
            // Parse docPatterns into an array of glob strings
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

    // No longer used now that the profile is a single free-form textarea,
    // but kept here for potential future expansion.

    return (
        <div className="view" style={{ width: '100%', maxWidth: '100%' }}>
            <section className="section" style={{ padding: '1.5rem' }}>
                {/* Header row */}
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: '1.5rem' }}>
                    {onBack && (
                        <button
                            onClick={onBack}
                            className="btn-secondary"
                            style={{ padding: '0.5rem 1rem', fontSize: '0.875rem' }}
                        >
                            ← Back to Sources
                        </button>
                    )}
                    {/* Stats on the right */}
                    {source?.stats && (
                        <div style={{ display: 'flex', gap: '2rem', fontSize: '0.9rem' }}>
                            <div style={{ textAlign: 'center' }}>
                                <div style={{ fontWeight: '600', color: 'var(--primary)', fontSize: '1.5rem' }}>{source.stats.file_count}</div>
                                <div style={{ color: 'var(--text-secondary)', fontSize: '0.8rem' }}>files</div>
                            </div>
                            <div style={{ textAlign: 'center' }}>
                                <div style={{ fontWeight: '600', color: 'var(--primary)', fontSize: '1.5rem' }}>{source.stats.chunk_count}</div>
                                <div style={{ color: 'var(--text-secondary)', fontSize: '0.8rem' }}>chunks</div>
                            </div>
                            <div style={{ textAlign: 'center' }}>
                                <div style={{ fontWeight: '600', color: 'var(--primary)', fontSize: '1.5rem' }}>{(source.stats.total_size_bytes / 1024).toFixed(0)}</div>
                                <div style={{ color: 'var(--text-secondary)', fontSize: '0.8rem' }}>KB</div>
                            </div>
                        </div>
                    )}
                </div>

                {/* Source Info */}
                {source && (
                    <div style={{ display: 'flex', alignItems: 'center', gap: '1rem', marginBottom: '1.5rem' }}>
                        <div style={{ fontSize: '2.5rem' }}>
                            {source.resource_type === 'local' ? '📁' : source.resource_type === 'git' ? '🔗' : source.resource_type === 'uploads' ? '📥' : '🌐'}
                        </div>
                        <div>
                            <h2 style={{ margin: 0, fontSize: '1.5rem' }}>{source.name}</h2>
                            <div style={{ display: 'flex', gap: '0.75rem', marginTop: '0.25rem', color: 'var(--text-secondary)', fontSize: '0.85rem', alignItems: 'center' }}>
                                <span className="badge">{source.resource_type.toUpperCase()}</span>
                                {source.resource_type !== 'uploads' && (
                                    <span title={source.path} style={{ opacity: 0.7 }}>📍 {source.path}</span>
                                )}
                            </div>
                        </div>
                    </div>
                )}

                {message && (
                    <div className={`status ${message.type}`} style={{ marginBottom: '1.5rem' }}>
                        {message.text}
                    </div>
                )}

                {/* File List section for uploads sources */}
                {source?.resource_type === 'uploads' && (
                    <>
                        {/* Hidden file input */}
                    <input
                        ref={fileInputRef}
                        type="file"
                        multiple
                        accept=".pdf,.docx,.doc,.txt,.md,.markdown,.json,.yaml,.yml,.toml,.csv,.xml,.html,.htm,.rst,.tex"
                        style={{ display: 'none' }}
                        onChange={handleFileChange}
                    />

                    {/* Drag & Drop Zone */}
                    <div
                        ref={dropZoneRef}
                        onDragEnter={handleDragEnter}
                        onDragLeave={handleDragLeave}
                        onDragOver={handleDragOver}
                        onDrop={handleDrop}
                        onClick={handleUploadClick}
                        style={{
                            border: `2px dashed ${isDragging ? 'var(--primary)' : 'var(--border)'}`,
                            borderRadius: '12px',
                            padding: '2rem',
                            textAlign: 'center',
                            cursor: uploading ? 'wait' : 'pointer',
                            background: isDragging ? 'rgba(100, 108, 255, 0.1)' : 'var(--surface)',
                            transition: 'all 0.2s ease',
                            marginBottom: '1.5rem',
                        }}
                    >
                        <div style={{ fontSize: '2.5rem', marginBottom: '0.5rem' }}>
                            {uploading ? '⏳' : isDragging ? '📥' : '📤'}
                        </div>
                        <div style={{ fontSize: '1rem', fontWeight: '500', color: 'var(--text)', marginBottom: '0.25rem' }}>
                            {uploading ? 'Uploading...' : isDragging ? 'Drop files here' : 'Drop files here or click to browse'}
                        </div>
                        <div style={{ fontSize: '0.875rem', color: 'var(--text-secondary)' }}>
                            Supports PDF, DOCX, TXT, MD, JSON, YAML, and more
                        </div>
                    </div>

                    {/* File List */}
                    {files.length > 0 && (
                        <>
                            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: '0.75rem' }}>
                                <h4 style={{ margin: 0, fontSize: '0.875rem', color: 'var(--text-secondary)', fontWeight: '500' }}>
                                    📁 {files.length} file{files.length !== 1 ? 's' : ''} uploaded
                                </h4>
                            </div>
                            <div style={{ background: 'var(--surface)', borderRadius: '8px', overflow: 'hidden' }}>
                                <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                                    <thead>
                                        <tr style={{ borderBottom: '1px solid var(--border)' }}>
                                            <th style={{ padding: '0.5rem 0.75rem', textAlign: 'left', fontWeight: '500', color: 'var(--text-secondary)', fontSize: '0.8rem' }}>Filename</th>
                                            <th style={{ padding: '0.5rem 0.75rem', textAlign: 'right', fontWeight: '500', color: 'var(--text-secondary)', fontSize: '0.8rem', width: '80px' }}>Chunks</th>
                                            <th style={{ padding: '0.5rem 0.75rem', textAlign: 'right', fontWeight: '500', color: 'var(--text-secondary)', fontSize: '0.8rem', width: '80px' }}></th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {files.map((file) => (
                                            <tr key={file.filename} style={{ borderBottom: '1px solid var(--border)' }}>
                                                <td style={{ padding: '0.5rem 0.75rem' }}>
                                                    <span style={{ fontFamily: 'monospace', fontSize: '0.85rem' }}>{file.filename}</span>
                                                </td>
                                                <td style={{ padding: '0.5rem 0.75rem', textAlign: 'right', color: 'var(--text-secondary)', fontSize: '0.85rem' }}>
                                                    {file.chunk_count}
                                                </td>
                                                <td style={{ padding: '0.5rem 0.75rem', textAlign: 'right' }}>
                                                    <button
                                                        onClick={(e) => { e.stopPropagation(); handleDeleteFile(file.filename); }}
                                                        disabled={deletingFile === file.filename}
                                                        style={{
                                                            background: 'transparent',
                                                            border: 'none',
                                                            color: deletingFile === file.filename ? 'var(--text-secondary)' : '#ef4444',
                                                            cursor: deletingFile === file.filename ? 'wait' : 'pointer',
                                                            padding: '0.25rem 0.5rem',
                                                            borderRadius: '4px',
                                                            fontSize: '0.8rem',
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
                    </>
                )}
            </section>

            {/* Profile section for non-uploads sources */}
            {source?.resource_type !== 'uploads' && (
                <section className="section" style={{ marginTop: '1rem' }}>
                    <div style={{ display: 'grid', gap: '1.5rem' }}>
                        <div className="form-group">
                            <label htmlFor="doc-patterns">
                                Doc File Patterns
                                <span style={{ color: 'var(--text-secondary)', fontSize: '0.875rem', marginLeft: '0.5rem' }}>
                                    Comma-separated globs (used to pick files for auto-generate)
                                </span>
                            </label>
                            <input
                                id="doc-patterns"
                                type="text"
                                value={docPatterns}
                                onChange={e => setDocPatterns(e.target.value)}
                                placeholder="e.g., *.md, README*, *.txt, *.toml, *.json"
                            />
                        </div>

                        <div style={{ display: 'flex', gap: '1rem' }}>
                            <button
                                onClick={handleGenerate}
                                disabled={generating || loading}
                                className="btn-primary"
                            >
                                {generating ? '⏳ Generating...' : '✨ Auto-Generate from Source'}
                            </button>
                            <button
                                onClick={handleSave}
                                disabled={loading}
                                className="btn-primary"
                            >
                                {loading ? '💾 Saving...' : '💾 Save Profile'}
                            </button>
                        </div>
                    </div>

                    <div style={{ display: 'grid', gap: '1.5rem', marginTop: '2rem' }}>
                        <div className="form-group">
                            <label htmlFor="profile-description">
                                Profile
                                <span style={{ color: 'var(--text-secondary)', fontSize: '0.875rem', marginLeft: '0.5rem' }}>
                                    Free-form project profile text (auto-generated or edited manually)
                                </span>
                            </label>
                            <textarea
                                id="profile-description"
                                value={profile.description}
                                onChange={e => setProfile(prev => ({ ...prev, description: e.target.value }))}
                                rows={10}
                                placeholder="High-level description of this source, tech stack, architecture, and conventions..."
                            />
                        </div>
                    </div>
                </section>
            )}
        </div>
    );
};
