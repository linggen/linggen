import React, { useEffect, useState, useCallback, useRef } from 'react';
import CodeMirror, { type ReactCodeMirrorRef } from '@uiw/react-codemirror';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';
import { oneDark } from '@codemirror/theme-one-dark';
import { EditorView } from '@codemirror/view';
import { getNote, saveNote, getMemoryFile, saveMemoryFile, getPack, savePack } from '../api';
import { livePreviewPlugin, livePreviewTheme } from './cm6-live-preview';

interface CM6EditorProps {
    sourceId: string;
    docPath: string;
    docType?: 'notes' | 'memory' | 'library';
    /**
     * How scrolling should work:
     * - editor: CodeMirror scrolls internally (default, best for large docs)
     * - container: editor grows with content; parent container scrolls
     */
    scrollMode?: 'editor' | 'container';
    onClose?: () => void;
}

// Custom theme to match Linggen dark mode
const linggenBaseTheme = EditorView.theme({
    '&': {
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

const linggenFillHeightTheme = EditorView.theme(
    {
        '&': { height: '100%' },
        '.cm-scroller': { height: '100%' },
    },
    { dark: true }
);

const linggenAutoHeightTheme = EditorView.theme(
    {
        '&': { height: 'auto' },
        '.cm-scroller': { overflow: 'visible' },
    },
    { dark: true }
);

export const CM6Editor: React.FC<CM6EditorProps> = ({
    sourceId,
    docPath,
    docType = 'notes',
    scrollMode = 'editor',
}) => {
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
                const doc =
                    docType === 'memory'
                        ? await getMemoryFile(sourceId, docPath)
                        : docType === 'library'
                        ? await getPack(docPath)
                        : await getNote(sourceId, docPath);
                setValue(doc.content || '');
                setDirty(false);
            } catch (err) {
                console.error("Failed to load note:", err);
                setValue("# Error loading document\n\n" + String(err));
            } finally {
                setLoading(false);
            }
        };
        load();
    }, [sourceId, docPath, docType]);

    // Save handler
    const handleSave = useCallback(async () => {
        if (!dirty) return;
        setSaving(true);
        try {
            if (docType === 'memory') {
                await saveMemoryFile(sourceId, docPath, value);
            } else if (docType === 'library') {
                await savePack(docPath, value);
            } else {
                await saveNote(sourceId, docPath, value);
            }
            setLastSaved(new Date());
            setDirty(false);
        } catch (err) {
            console.error("Failed to save note:", err);
        } finally {
            setSaving(false);
        }
    }, [dirty, sourceId, docPath, docType, value]);

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
        return (
            <div className="flex items-center justify-center h-full text-[var(--text-muted)] text-[0.9rem] bg-[var(--bg-content)]">
                Loading {docPath}...
            </div>
        );
    }

    const isContainerScroll = scrollMode === 'container';

    return (
        <div className={`flex flex-col h-full min-h-0 bg-[var(--bg-content)] overflow-hidden ${isContainerScroll ? 'h-auto overflow-visible' : ''}`}>
            {/* Header / Status Bar */}
            <div className="flex justify-between items-center px-4 py-2 border-b border-[var(--border-color)] bg-[var(--bg-content)]">
                <div className="text-[0.9rem] font-medium text-[var(--text-primary)]">
                    {docType === 'memory' ? `Memory: ${docPath}` : docType === 'library' ? `Library: ${docPath}` : docPath}
                </div>
                <div className="text-[0.8rem] text-[var(--text-muted)]">
                    {saving ? (
                        <span className="text-amber-500">Saving...</span>
                    ) : lastSaved ? (
                        <span className="text-green-500">Saved {lastSaved.toLocaleTimeString()}</span>
                    ) : dirty ? (
                        <span className="text-amber-500">Unsaved</span>
                    ) : null}
                </div>
            </div>

            {/* Editor */}
            <div className={`min-h-0 ${isContainerScroll ? 'block overflow-visible' : 'flex-1 overflow-hidden'}`}>
                <CodeMirror
                    ref={editorRef}
                    value={value}
                    onChange={handleChange}
                    height={scrollMode === 'editor' ? '100%' : 'auto'}
                    minHeight="200px"
                    className={isContainerScroll ? 'cm-auto-height' : 'h-full'}
                    theme={oneDark}
                    extensions={[
                        markdown({
                            base: markdownLanguage,
                            codeLanguages: languages
                        }),
                        linggenBaseTheme,
                        scrollMode === 'editor' ? linggenFillHeightTheme : linggenAutoHeightTheme,
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
