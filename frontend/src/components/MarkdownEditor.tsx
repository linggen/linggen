import React, { useEffect, useState } from 'react';
import MDEditor from '@uiw/react-md-editor';
import { getNote, saveNote } from '../api';

interface MarkdownEditorProps {
    sourceId: string;
    notePath: string; // Relative path, e.g., "Note.md"
    onClose?: () => void;
}

export const MarkdownEditor: React.FC<MarkdownEditorProps> = ({ sourceId, notePath }) => {
    const [value, setValue] = useState<string>('');
    const [loading, setLoading] = useState(false);
    const [saving, setSaving] = useState(false);
    const [dirty, setDirty] = useState(false);
    const [lastSaved, setLastSaved] = useState<Date | null>(null);

    // Initial load
    useEffect(() => {
        const load = async () => {
            setLoading(true);
            try {
                const note = await getNote(sourceId, notePath);
                setValue(note.content || '');
                setDirty(false);
            } catch (err) {
                console.error("Failed to load note:", err);
                setValue("# Error loading note\n\n" + String(err));
            } finally {
                setLoading(false);
            }
        };
        load();
    }, [sourceId, notePath]);

    // Save handler
    const handleSave = async () => {
        if (!dirty) return;
        setSaving(true);
        try {
            await saveNote(sourceId, notePath, value);
            setLastSaved(new Date());
            setDirty(false);
        } catch (err) {
            console.error("Failed to save note:", err);
            // Optionally show toast
        } finally {
            setSaving(false);
        }
    };

    // Auto-save debounce could go here, or manual save for now.
    // Let's implement Cmd+S support
    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            if ((e.metaKey || e.ctrlKey) && e.key === 's') {
                e.preventDefault();
                handleSave();
            }
        };
        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [value, dirty, sourceId, notePath]); // Dep list important for closure

    if (loading) {
        return <div className="editor-loading">Loading {notePath}...</div>;
    }

    return (
        <div style={{ height: '100%', display: 'flex', flexDirection: 'column', background: 'var(--bg-content)', position: 'relative' }}>
            {/* Header / Toolbar */}
            <div style={{
                padding: '8px 16px',
                borderBottom: '1px solid var(--border-color)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                background: 'var(--bg-content)'
            }}>
                <div></div>

                <div style={{ display: 'flex', alignItems: 'center', gap: '12px', fontSize: '0.8rem', color: 'var(--text-muted)' }}>
                    {saving ? (
                        <span>Saving...</span>
                    ) : lastSaved ? (
                        <span>Saved {lastSaved.toLocaleTimeString()}</span>
                    ) : null}
                    {/* Add more toolbar items if needed */}
                </div>
            </div>

            {/* Editor Area */}
            <div style={{ flex: 1, overflow: 'hidden' }} data-color-mode="dark">
                <MDEditor
                    value={value}
                    onChange={(val) => {
                        setValue(val || '');
                        setDirty(true);
                    }}
                    height="100%"
                    visibleDragbar={false}
                    preview="live"
                    style={{ background: 'var(--bg-content)', border: 'none', color: 'var(--text-primary)' }}
                />
            </div>
        </div>
    );
};
