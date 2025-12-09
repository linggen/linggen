import React, { useEffect, useState, useCallback, useRef } from 'react';
import CodeMirror, { type ReactCodeMirrorRef } from '@uiw/react-codemirror';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';
import { oneDark } from '@codemirror/theme-one-dark';
import { EditorView } from '@codemirror/view';
import { getNote, saveNote } from '../api';
import { livePreviewPlugin, livePreviewTheme } from './cm6-live-preview';
import './CM6Editor.css';

interface CM6EditorProps {
    sourceId: string;
    notePath: string;
    onClose?: () => void;
}

// Custom theme to match Linggen dark mode
const linggenTheme = EditorView.theme({
    '&': {
        height: '100%',
        fontSize: '14px',
    },
    '.cm-content': {
        fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
        padding: '16px',
    },
    '.cm-line': {
        padding: '0 4px',
    },
    '.cm-gutters': {
        backgroundColor: 'transparent',
        borderRight: '1px solid var(--border-color, #333)',
    },
    '.cm-activeLineGutter': {
        backgroundColor: 'rgba(255, 255, 255, 0.05)',
    },
    '.cm-activeLine': {
        backgroundColor: 'rgba(255, 255, 255, 0.03)',
    },
    '.cm-selectionBackground': {
        backgroundColor: 'rgba(100, 108, 255, 0.3) !important',
    },
    '.cm-cursor': {
        borderLeftColor: '#fff',
    },
    // Markdown-specific styling
    '.cm-header-1': {
        fontSize: '1.8em',
        fontWeight: 'bold',
        color: '#e2e8f0',
    },
    '.cm-header-2': {
        fontSize: '1.5em',
        fontWeight: 'bold',
        color: '#e2e8f0',
    },
    '.cm-header-3': {
        fontSize: '1.3em',
        fontWeight: 'bold',
        color: '#e2e8f0',
    },
    '.cm-header-4, .cm-header-5, .cm-header-6': {
        fontSize: '1.1em',
        fontWeight: 'bold',
        color: '#e2e8f0',
    },
    '.cm-strong': {
        fontWeight: 'bold',
        color: '#f8fafc',
    },
    '.cm-emphasis': {
        fontStyle: 'italic',
        color: '#cbd5e1',
    },
    '.cm-strikethrough': {
        textDecoration: 'line-through',
    },
    '.cm-link': {
        color: '#60a5fa',
        textDecoration: 'underline',
    },
    '.cm-url': {
        color: '#94a3b8',
    },
    '.cm-code': {
        backgroundColor: 'rgba(100, 108, 255, 0.1)',
        color: '#a78bfa',
        padding: '2px 4px',
        borderRadius: '3px',
    },
}, { dark: true });

export const CM6Editor: React.FC<CM6EditorProps> = ({ sourceId, notePath }) => {
    const [value, setValue] = useState<string>('');
    const [loading, setLoading] = useState(false);
    const [saving, setSaving] = useState(false);
    const [dirty, setDirty] = useState(false);
    const [lastSaved, setLastSaved] = useState<Date | null>(null);
    const editorRef = useRef<ReactCodeMirrorRef>(null);

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

    const handleChange = useCallback((val: string) => {
        setValue(val);
        setDirty(true);
    }, []);

    if (loading) {
        return <div className="cm6-editor-loading">Loading {notePath}...</div>;
    }

    return (
        <div className="cm6-editor-container">
            {/* Header / Status Bar */}
            <div className="cm6-editor-header">
                <div className="cm6-editor-title">{notePath}</div>
                <div className="cm6-editor-status">
                    {saving ? (
                        <span className="saving">Saving...</span>
                    ) : lastSaved ? (
                        <span className="saved">Saved {lastSaved.toLocaleTimeString()}</span>
                    ) : dirty ? (
                        <span className="unsaved">Unsaved</span>
                    ) : null}
                </div>
            </div>

            {/* Editor */}
            <div className="cm6-editor-content">
                <CodeMirror
                    ref={editorRef}
                    value={value}
                    onChange={handleChange}
                    height="100%"
                    
                    theme={oneDark}
                    extensions={[
                        markdown({
                            base: markdownLanguage,
                            codeLanguages: languages
                        }),
                        linggenTheme,
                        livePreviewPlugin,
                        livePreviewTheme,
                        EditorView.lineWrapping,
                    ]}
                    basicSetup={{
                        lineNumbers: false,
                        foldGutter: true,
                        highlightActiveLineGutter: true,
                        highlightActiveLine: true,
                        bracketMatching: true,
                        closeBrackets: true,
                        autocompletion: true,
                        history: true,
                        searchKeymap: true,
                    }}
                />
            </div>
        </div>
    );
};
