import React, { useState } from 'react';
import type { ResourceType } from '../api';

interface AddSourceModalProps {
    isOpen: boolean;
    onClose: () => void;
    onAdd: (name: string, type: ResourceType, path: string, include?: string[], exclude?: string[]) => Promise<void>;
}

export const AddSourceModal: React.FC<AddSourceModalProps> = ({ isOpen, onClose, onAdd }) => {
    const [name, setName] = useState('');
    const [type, setType] = useState<ResourceType>('local');
    const [path, setPath] = useState('');
    const [includePatterns, setIncludePatterns] = useState('');
    const [excludePatterns, setExcludePatterns] = useState('');
    const [isSubmitting, setIsSubmitting] = useState(false);
    const [error, setError] = useState<string | null>(null);

    if (!isOpen) return null;

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        setError(null);

        if (!name.trim()) {
            setError('Name is required');
            return;
        }
        if (!path.trim() && type !== 'uploads') {
            setError(type === 'git' ? 'Git URL is required' : 'Path is required');
            return;
        }

        try {
            setIsSubmitting(true);

            const includes = includePatterns.split(',').map(s => s.trim()).filter(Boolean);
            const excludes = excludePatterns.split(',').map(s => s.trim()).filter(Boolean);

            await onAdd(name, type, path, includes, excludes);
            // Reset and close on success
            setName('');
            setType('local');
            setPath('');
            setIncludePatterns('');
            setExcludePatterns('');
            onClose();
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Failed to add source');
        } finally {
            setIsSubmitting(false);
        }
    };

    return (
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
            backdropFilter: 'blur(2px)'
        }} onClick={onClose}>
            <div className="modal-content" style={{
                backgroundColor: 'var(--bg-sidebar)',
                borderRadius: '8px',
                padding: '24px',
                width: '450px',
                maxWidth: '90%',
                border: '1px solid var(--border-color)',
                boxShadow: '0 4px 20px rgba(0, 0, 0, 0.3)'
            }} onClick={e => e.stopPropagation()}>
                <h3 style={{ marginTop: 0, marginBottom: '20px', fontSize: '1.1rem', color: 'var(--text-active)' }}>
                    Add New Source
                </h3>

                <form onSubmit={handleSubmit}>
                    <div className="form-group" style={{ marginBottom: '16px' }}>
                        <label style={{ display: 'block', marginBottom: '6px', fontSize: '0.85rem', color: 'var(--text-secondary)' }}>
                            Name
                        </label>
                        <input
                            type="text"
                            value={name}
                            onChange={e => setName(e.target.value)}
                            placeholder="e.g. My Project"
                            autoFocus
                            style={{
                                width: '100%',
                                padding: '8px 12px',
                                borderRadius: '4px',
                                border: '1px solid var(--border-color)',
                                background: 'var(--bg-app)',
                                color: 'var(--text-primary)',
                                outline: 'none'
                            }}
                        />
                    </div>

                    <div className="form-group" style={{ marginBottom: '16px' }}>
                        <label style={{ display: 'block', marginBottom: '6px', fontSize: '0.85rem', color: 'var(--text-secondary)' }}>
                            Type
                        </label>
                        <select
                            value={type}
                            onChange={e => setType(e.target.value as ResourceType)}
                            style={{
                                width: '100%',
                                padding: '8px 12px',
                                borderRadius: '4px',
                                border: '1px solid var(--border-color)',
                                background: 'var(--bg-app)',
                                color: 'var(--text-primary)',
                                outline: 'none'
                            }}
                        >
                            <option value="local">Local Folder</option>
                            <option value="git">Git Repository</option>
                            <option value="uploads">Upload Files</option>
                            <option value="web" disabled>Web Page (Coming Soon)</option>
                        </select>
                    </div>

                    {type !== 'uploads' && (
                        <div className="form-group" style={{ marginBottom: '24px' }}>
                            <label style={{ display: 'block', marginBottom: '6px', fontSize: '0.85rem', color: 'var(--text-secondary)' }}>
                                {type === 'git' ? 'Git URL' : 'Local Path'}
                            </label>
                            <input
                                type="text"
                                value={path}
                                onChange={e => setPath(e.target.value)}
                                placeholder={type === 'git' ? 'https://github.com/user/repo' : '/absolute/path/to/project'}
                                style={{
                                    width: '100%',
                                    padding: '8px 12px',
                                    borderRadius: '4px',
                                    border: '1px solid var(--border-color)',
                                    background: 'var(--bg-app)',
                                    color: 'var(--text-primary)',
                                    outline: 'none',
                                    fontFamily: 'monospace'
                                }}
                            />
                        </div>
                    )}

                    <div style={{ display: 'flex', gap: '16px', marginBottom: '16px' }}>
                        <div className="form-group" style={{ flex: 1 }}>
                            <label style={{ display: 'block', marginBottom: '6px', fontSize: '0.85rem', color: 'var(--text-secondary)' }}>
                                Include Patterns (Optional)
                            </label>
                            <input
                                type="text"
                                value={includePatterns}
                                onChange={e => setIncludePatterns(e.target.value)}
                                placeholder="e.g. *.rs, src/**/*.ts"
                                style={{
                                    width: '100%',
                                    padding: '8px 12px',
                                    borderRadius: '4px',
                                    border: '1px solid var(--border-color)',
                                    background: 'var(--bg-app)',
                                    color: 'var(--text-primary)',
                                    outline: 'none',
                                    fontFamily: 'monospace',
                                    fontSize: '0.8rem'
                                }}
                            />
                            <div style={{ fontSize: '0.7rem', color: 'var(--text-muted)', marginTop: '4px' }}>
                                Glob patterns, comma separated
                            </div>
                        </div>

                        <div className="form-group" style={{ flex: 1 }}>
                            <label style={{ display: 'block', marginBottom: '6px', fontSize: '0.85rem', color: 'var(--text-secondary)' }}>
                                Exclude Patterns (Optional)
                            </label>
                            <input
                                type="text"
                                value={excludePatterns}
                                onChange={e => setExcludePatterns(e.target.value)}
                                placeholder="e.g. target/*, node_modules/*"
                                style={{
                                    width: '100%',
                                    padding: '8px 12px',
                                    borderRadius: '4px',
                                    border: '1px solid var(--border-color)',
                                    background: 'var(--bg-app)',
                                    color: 'var(--text-primary)',
                                    outline: 'none',
                                    fontFamily: 'monospace',
                                    fontSize: '0.8rem'
                                }}
                            />
                        </div>
                    </div>

                    {error && (
                        <div style={{
                            color: '#f48771',
                            fontSize: '0.85rem',
                            marginBottom: '16px',
                            background: 'rgba(244, 135, 113, 0.1)',
                            padding: '8px',
                            borderRadius: '4px'
                        }}>
                            {error}
                        </div>
                    )}

                    <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '12px' }}>
                        <button
                            type="button"
                            onClick={onClose}
                            style={{
                                padding: '8px 16px',
                                background: 'transparent',
                                border: '1px solid var(--border-color)',
                                borderRadius: '4px',
                                color: 'var(--text-primary)',
                                cursor: 'pointer'
                            }}
                        >
                            Cancel
                        </button>
                        <button
                            type="submit"
                            disabled={isSubmitting}
                            style={{
                                padding: '8px 16px',
                                background: 'var(--accent)',
                                border: '1px solid var(--accent)',
                                borderRadius: '4px',
                                color: 'white',
                                cursor: isSubmitting ? 'not-allowed' : 'pointer',
                                opacity: isSubmitting ? 0.7 : 1
                            }}
                        >
                            {isSubmitting ? 'Adding...' : 'Add Source'}
                        </button>
                    </div>
                </form>
            </div>
        </div>
    );
};
