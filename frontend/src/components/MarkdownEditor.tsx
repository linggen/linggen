import React, { useEffect, useState, useCallback } from 'react';
import MDEditor from '@uiw/react-md-editor';
import mermaid from 'mermaid';
import { getNote, saveNote } from '../api';
import './MarkdownEditor.css';

// Initialize mermaid with dark theme
mermaid.initialize({
    startOnLoad: false,
    theme: 'dark',
    securityLevel: 'loose',
});

// Mermaid renderer component
const MermaidBlock: React.FC<{ code: string }> = ({ code }) => {
    const [svg, setSvg] = useState<string>('');
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        const renderMermaid = async () => {
            try {
                // Clean up the code - trim whitespace and normalize line endings
                const cleanCode = code
                    .trim()
                    .replace(/\r\n/g, '\n')
                    .replace(/^\s+/gm, (match) => match.replace(/\t/g, '  ')); // normalize tabs

                const id = `mermaid-${Math.random().toString(36).substr(2, 9)}`;
                const { svg } = await mermaid.render(id, cleanCode);
                setSvg(svg);
                setError(null);
            } catch (err) {
                console.error('Mermaid render error:', err);
                setError(err instanceof Error ? err.message : 'Failed to render diagram');
            }
        };

        if (code && code.trim()) {
            renderMermaid();
        }
    }, [code]);

    if (error) {
        return (
            <div style={{
                background: 'rgba(239, 68, 68, 0.1)',
                border: '1px solid rgba(239, 68, 68, 0.3)',
                borderRadius: '4px',
                padding: '8px 12px',
                color: '#ef4444',
                fontSize: '0.85rem'
            }}>
                Mermaid Error: {error}
            </div>
        );
    }

    return <div dangerouslySetInnerHTML={{ __html: svg }} style={{ display: 'flex', justifyContent: 'center' }} />;
};

// Helper function to extract text from React children
const getTextFromChildren = (children: any): string => {
    if (typeof children === 'string') {
        return children;
    }
    if (Array.isArray(children)) {
        return children.map(getTextFromChildren).join('');
    }
    if (children?.props?.children) {
        return getTextFromChildren(children.props.children);
    }
    if (children?.props?.node?.value) {
        return children.props.node.value;
    }
    return '';
};

// Custom code block renderer that handles mermaid
const CodeBlock = ({ inline, children, className, node, ...props }: any) => {
    // Try to get raw value from the AST node first, then fallback to extracting from children
    const code = node?.children?.[0]?.value || getTextFromChildren(children);
    const cleanCode = code.replace(/\n$/, '');

    const match = /language-(\w+)/.exec(className || '');
    const language = match ? match[1] : '';

    if (!inline && language === 'mermaid' && cleanCode) {
        return <MermaidBlock code={cleanCode} />;
    }

    return (
        <code className={className} {...props}>
            {children}
        </code>
    );
};

interface MarkdownEditorProps {
    sourceId: string;
    notePath: string;
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
    const handleSave = useCallback(async () => {
        if (!dirty) return;
        setSaving(true);
        try {
            await saveNote(sourceId, notePath, value);
            setLastSaved(new Date());
            setDirty(false);
        } catch (err) {
            console.error("Failed to save note:", err);
        } finally {
            setSaving(false);
        }
    }, [dirty, sourceId, notePath, value]);

    // Autosave with debounce (1.5 seconds after typing stops)
    useEffect(() => {
        if (!dirty) return;

        const timer = setTimeout(() => {
            handleSave();
        }, 1500);

        return () => clearTimeout(timer);
    }, [value, dirty, handleSave]);

    // Cmd+S support
    useEffect(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            if ((e.metaKey || e.ctrlKey) && e.key === 's') {
                e.preventDefault();
                handleSave();
            }
        };
        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [handleSave]);

    if (loading) {
        return <div className="editor-loading">Loading {notePath}...</div>;
    }

    return (
        <div className="markdown-editor-container" style={{ height: '100%', display: 'flex', flexDirection: 'column', background: 'var(--bg-content)', position: 'relative' }}>
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
                    previewOptions={{
                        components: {
                            code: CodeBlock
                        }
                    }}
                />
            </div>
        </div>
    );
};

