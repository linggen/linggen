import React, { useState } from 'react';
import type { ResourceType } from '../api';

interface AddSourceModalProps {
    isOpen: boolean;
    onClose: () => void;
    onAdd: (name: string, type: ResourceType, path: string, include?: string[], exclude?: string[]) => Promise<void>;
}

export const AddSourceModal: React.FC<AddSourceModalProps> = ({ isOpen, onClose, onAdd }) => {
    const [name, setName] = useState('');
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
        if (!path.trim()) {
            setError('Path is required');
            return;
        }

        try {
            setIsSubmitting(true);

            const includes = includePatterns.split(',').map(s => s.trim()).filter(Boolean);
            const excludes = excludePatterns.split(',').map(s => s.trim()).filter(Boolean);

            // Always use 'local' type for now
            await onAdd(name, 'local', path, includes, excludes);
            // Reset and close on success
            setName('');
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
        <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-[1000] backdrop-blur-[2px]" onClick={onClose}>
            <div className="bg-[var(--bg-sidebar)] rounded-lg p-6 w-[450px] max-w-[90%] border border-[var(--border-color)] shadow-[0_4px_20px_rgba(0,0,0,0.3)]" onClick={e => e.stopPropagation()}>
                <h3 className="mt-0 mb-5 text-[1.1rem] font-semibold text-[var(--text-active)] uppercase tracking-wider">
                    Add New Project
                </h3>

                <form onSubmit={handleSubmit}>
                    <div className="mb-4">
                        <label className="block mb-1.5 text-[0.85rem] font-semibold text-[var(--text-secondary)] uppercase tracking-wide">
                            NAME
                        </label>
                        <input
                            type="text"
                            value={name}
                            onChange={e => setName(e.target.value)}
                            placeholder="e.g. My Project"
                            autoFocus
                            className="w-full px-3 py-2 rounded border border-[var(--border-color)] bg-[var(--bg-app)] text-[var(--text-primary)] outline-none focus:border-[var(--accent)] transition-colors"
                        />
                    </div>

                    <div className="mb-4">
                        <label className="block mb-1.5 text-[0.85rem] font-semibold text-[var(--text-secondary)] uppercase tracking-wide">
                            TYPE
                        </label>
                        <div className="w-full px-3 py-2 rounded border border-[var(--border-color)] bg-[var(--bg-content)] text-[var(--text-primary)] flex items-center text-[0.9rem]">
                            ✓ Local Folder
                            <span className="ml-auto text-[0.75rem] text-[var(--text-muted)] italic">
                                (Other types coming soon)
                            </span>
                        </div>
                    </div>

                    <div className="mb-6">
                        <label className="block mb-1.5 text-[0.85rem] font-semibold text-[var(--text-secondary)] uppercase tracking-wide">
                            LOCAL PATH
                        </label>
                        <input
                            type="text"
                            value={path}
                            onChange={e => setPath(e.target.value)}
                            placeholder="/absolute/path/to/project"
                            className="w-full px-3 py-2 rounded border border-[var(--border-color)] bg-[var(--bg-app)] text-[var(--text-primary)] outline-none focus:border-[var(--accent)] font-mono transition-colors"
                        />
                    </div>

                    <div className="flex gap-4 mb-4">
                        <div className="flex-1">
                            <label className="block mb-1.5 text-[0.85rem] font-semibold text-[var(--text-secondary)] uppercase tracking-wide">
                                INCLUDE PATTERNS (OPTIONAL)
                            </label>
                            <input
                                type="text"
                                value={includePatterns}
                                onChange={e => setIncludePatterns(e.target.value)}
                                placeholder="e.g. *.rs, src/**/*.ts"
                                className="w-full px-3 py-2 rounded border border-[var(--border-color)] bg-[var(--bg-app)] text-[var(--text-primary)] outline-none focus:border-[var(--accent)] font-mono text-[0.8rem] transition-colors"
                            />
                            <div className="text-[0.7rem] text-[var(--text-muted)] mt-1">
                                Glob patterns, comma separated
                            </div>
                        </div>

                        <div className="flex-1">
                            <label className="block mb-1.5 text-[0.85rem] font-semibold text-[var(--text-secondary)] uppercase tracking-wide">
                                EXCLUDE PATTERNS (OPTIONAL)
                            </label>
                            <input
                                type="text"
                                value={excludePatterns}
                                onChange={e => setExcludePatterns(e.target.value)}
                                placeholder="e.g. target/*, node_modules/*"
                                className="w-full px-3 py-2 rounded border border-[var(--border-color)] bg-[var(--bg-app)] text-[var(--text-primary)] outline-none focus:border-[var(--accent)] font-mono text-[0.8rem] transition-colors"
                            />
                        </div>
                    </div>

                    {error && (
                        <div className="text-[#f48771] text-[0.85rem] mb-4 bg-[#f48771]/10 p-2 rounded">
                            {error}
                        </div>
                    )}

                    <div className="flex justify-end gap-3 mt-6">
                        <button
                            type="button"
                            onClick={onClose}
                            className="btn-outline px-4 py-2 text-[12px]"
                        >
                            Cancel
                        </button>
                        <button
                            type="submit"
                            disabled={isSubmitting}
                            className={`btn-primary px-4 py-2 text-[12px] ${isSubmitting ? 'opacity-70 cursor-not-allowed' : ''}`}
                        >
                            {isSubmitting ? 'Adding...' : 'Add Project'}
                        </button>
                    </div>
                </form>
            </div>
        </div>
    );
};
