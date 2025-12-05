import { useState, useEffect, useCallback } from 'react';
import MDEditor from '@uiw/react-md-editor';
import './DesignEditor.css';

interface DesignEditorProps {
  sourceId: string;
  notePath: string | null;
  onNoteChange?: (path: string) => void;
  onFocusNode?: (nodeId: string) => void;
}

interface NoteInfo {
  path: string;
  name: string;
  modified_at?: string;
}

export function DesignEditor({ sourceId, notePath, onNoteChange, onFocusNode }: DesignEditorProps) {
  const [notes, setNotes] = useState<NoteInfo[]>([]);
  const [content, setContent] = useState<string>('');
  const [originalContent, setOriginalContent] = useState<string>('');
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showNewNoteInput, setShowNewNoteInput] = useState(false);
  const [newNoteName, setNewNoteName] = useState('');
  const [linkedNode, setLinkedNode] = useState<string | null>(null);

  // Load notes list
  const loadNotes = useCallback(async () => {
    try {
      const response = await fetch(`http://localhost:8787/api/sources/${sourceId}/notes`);
      if (response.ok) {
        const data = await response.json();
        setNotes(data.notes || []);
      } else if (response.status === 404) {
        // No notes yet, that's fine
        setNotes([]);
      }
    } catch (err) {
      console.error('Failed to load notes:', err);
    }
  }, [sourceId]);

  // Load specific note content
  const loadNote = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
    try {
      const response = await fetch(`http://localhost:8787/api/sources/${sourceId}/notes/${encodeURIComponent(path)}`);
      if (response.ok) {
        const data = await response.json();
        setContent(data.content || '');
        setOriginalContent(data.content || '');
        setLinkedNode(data.linked_node || null);
      } else if (response.status === 404) {
        // New note
        setContent('');
        setOriginalContent('');
        setLinkedNode(null);
      } else {
        setError('Failed to load note');
      }
    } catch (err) {
      setError(`Error: ${err}`);
    } finally {
      setLoading(false);
    }
  }, [sourceId]);

  // Save note
  const saveNote = useCallback(async () => {
    if (!notePath) return;
    
    setSaving(true);
    setError(null);
    try {
      const response = await fetch(`http://localhost:8787/api/sources/${sourceId}/notes/${encodeURIComponent(notePath)}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ content, linked_node: linkedNode }),
      });
      
      if (response.ok) {
        setOriginalContent(content);
        loadNotes(); // Refresh list
      } else {
        setError('Failed to save note');
      }
    } catch (err) {
      setError(`Error: ${err}`);
    } finally {
      setSaving(false);
    }
  }, [sourceId, notePath, content, linkedNode, loadNotes]);

  // Create new note
  const createNote = useCallback(async () => {
    if (!newNoteName.trim()) return;
    
    const path = newNoteName.endsWith('.md') ? newNoteName : `${newNoteName}.md`;
    
    try {
      await fetch(`http://localhost:8787/api/sources/${sourceId}/notes/${encodeURIComponent(path)}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ content: `# ${newNoteName.replace('.md', '')}\n\n` }),
      });
      
      setNewNoteName('');
      setShowNewNoteInput(false);
      loadNotes();
      onNoteChange?.(path);
    } catch (err) {
      setError(`Error creating note: ${err}`);
    }
  }, [sourceId, newNoteName, loadNotes, onNoteChange]);

  // Load notes on mount
  useEffect(() => {
    loadNotes();
  }, [loadNotes]);

  // Load note content when path changes
  useEffect(() => {
    if (notePath) {
      loadNote(notePath);
    } else {
      setContent('');
      setOriginalContent('');
      setLinkedNode(null);
    }
  }, [notePath, loadNote]);

  // Keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 's') {
        e.preventDefault();
        saveNote();
      }
    };
    
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [saveNote]);

  const hasChanges = content !== originalContent;

  return (
    <div className="design-editor">
      {/* Toolbar */}
      <div className="design-editor-toolbar">
        <div className="design-editor-toolbar-left">
          <select
            value={notePath || ''}
            onChange={(e) => onNoteChange?.(e.target.value)}
            className="design-editor-note-select"
          >
            <option value="">Select a note...</option>
            {notes.map((note) => (
              <option key={note.path} value={note.path}>
                {note.name}
              </option>
            ))}
          </select>
          
          {showNewNoteInput ? (
            <div className="design-editor-new-note">
              <input
                type="text"
                value={newNoteName}
                onChange={(e) => setNewNoteName(e.target.value)}
                placeholder="note-name.md"
                autoFocus
                onKeyDown={(e) => {
                  if (e.key === 'Enter') createNote();
                  if (e.key === 'Escape') {
                    setShowNewNoteInput(false);
                    setNewNoteName('');
                  }
                }}
              />
              <button onClick={createNote} className="btn-small btn-primary">Create</button>
              <button onClick={() => { setShowNewNoteInput(false); setNewNoteName(''); }} className="btn-small">Cancel</button>
            </div>
          ) : (
            <button onClick={() => setShowNewNoteInput(true)} className="btn-small">+ New Note</button>
          )}
        </div>
        
        <div className="design-editor-toolbar-right">
          {linkedNode && (
            <button 
              onClick={() => onFocusNode?.(linkedNode)}
              className="btn-small btn-link"
              title={`Linked to: ${linkedNode}`}
            >
              📍 {linkedNode.split('/').pop()}
            </button>
          )}
          
          {hasChanges && <span className="unsaved-indicator">Unsaved changes</span>}
          
          <button
            onClick={saveNote}
            disabled={!notePath || saving || !hasChanges}
            className={`btn-small btn-primary ${saving ? 'saving' : ''}`}
          >
            {saving ? 'Saving...' : '💾 Save'}
          </button>
        </div>
      </div>

      {/* Error message */}
      {error && (
        <div className="design-editor-error">
          {error}
        </div>
      )}

      {/* Editor */}
      <div className="design-editor-content" data-color-mode="dark">
        {loading ? (
          <div className="design-editor-loading">
            <div className="loading-spinner"></div>
            <p>Loading...</p>
          </div>
        ) : notePath ? (
          <MDEditor
            value={content}
            onChange={(val) => setContent(val || '')}
            height="100%"
            preview="live"
            hideToolbar={false}
            enableScroll={true}
          />
        ) : (
          <div className="design-editor-empty">
            <div className="empty-icon">📝</div>
            <h3>No note selected</h3>
            <p>Select an existing note or create a new one to start designing.</p>
            <button onClick={() => setShowNewNoteInput(true)} className="btn-primary">
              + Create New Note
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
