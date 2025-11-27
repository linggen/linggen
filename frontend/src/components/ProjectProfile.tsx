import React, { useEffect, useState } from 'react';
import { getProfile, updateProfile, generateProfile, type SourceProfile as SourceProfileType, type Resource } from '../api';

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

    useEffect(() => {
        loadData();
    }, []);

    const loadData = async () => {
        try {
            setLoading(true);

            // Load both profile and source info
            const [profileData, sourcesResponse] = await Promise.all([
                getProfile(sourceId),
                fetch(`http://localhost:3000/api/resources`).then(r => r.json())
            ]);

            setProfile(profileData);
            const foundSource = sourcesResponse.resources.find((r: Resource) => r.id === sourceId);
            if (foundSource) {
                setSource(foundSource);
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
        <div className="view">
            <div className="view-header">
                {onBack && (
                    <button
                        onClick={onBack}
                        className="btn-secondary"
                        style={{ marginBottom: '1rem' }}
                    >
                        ← Back to Sources
                    </button>
                )}
                <h2>{source ? `Source Profile: ${source.name}` : 'Source Profile'}</h2>
            </div>

            {source && (
                <section className="section" style={{ marginBottom: '1rem', background: 'var(--surface-secondary)' }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: '1rem' }}>
                        <div style={{ fontSize: '2rem' }}>
                            {source.resource_type === 'local' ? '📁' : source.resource_type === 'git' ? '🔗' : '🌐'}
                        </div>
                        <div style={{ flex: 1 }}>
                            <h3 style={{ margin: 0, fontSize: '1.25rem' }}>{source.name}</h3>
                            <div style={{ display: 'flex', gap: '1rem', marginTop: '0.25rem', color: 'var(--text-secondary)', fontSize: '0.875rem' }}>
                                <span className="badge">{source.resource_type.toUpperCase()}</span>
                                <span title={source.path}>📍 {source.path}</span>
                            </div>
                        </div>
                    </div>
                </section>
            )}

            {source && source.stats && (
                <section className="section" style={{ marginBottom: '1rem', background: 'var(--surface-secondary)' }}>
                    <h3 style={{ margin: '0 0 1rem 0', fontSize: '1rem', color: 'var(--text-secondary)' }}>📊 Indexing Statistics</h3>
                    <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(150px, 1fr))', gap: '1rem' }}>
                        <div style={{ padding: '1rem', background: 'var(--surface)', borderRadius: '8px' }}>
                            <div style={{ fontSize: '0.875rem', color: 'var(--text-secondary)', marginBottom: '0.25rem' }}>Files Indexed</div>
                            <div style={{ fontSize: '1.5rem', fontWeight: '600', color: 'var(--primary)' }}>
                                {source.stats.file_count.toLocaleString()}
                            </div>
                        </div>
                        <div style={{ padding: '1rem', background: 'var(--surface)', borderRadius: '8px' }}>
                            <div style={{ fontSize: '0.875rem', color: 'var(--text-secondary)', marginBottom: '0.25rem' }}>Chunks Created</div>
                            <div style={{ fontSize: '1.5rem', fontWeight: '600', color: 'var(--primary)' }}>
                                {source.stats.chunk_count.toLocaleString()}
                            </div>
                        </div>
                        <div style={{ padding: '1rem', background: 'var(--surface)', borderRadius: '8px' }}>
                            <div style={{ fontSize: '0.875rem', color: 'var(--text-secondary)', marginBottom: '0.25rem' }}>Total Size</div>
                            <div style={{ fontSize: '1.5rem', fontWeight: '600', color: 'var(--primary)' }}>
                                {(source.stats.total_size_bytes / 1024).toFixed(2)} KB
                            </div>
                        </div>
                    </div>
                </section>
            )}

            <section className="section">
                {message && (
                    <div className={`status ${message.type}`} style={{ marginBottom: '1.5rem' }}>
                        {message.text}
                    </div>
                )}

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
        </div>
    );
};
