import {
    FolderIcon,
    ChatBubbleLeftRightIcon,
    ClockIcon,
    Cog6ToothIcon,
    FolderPlusIcon,
    DocumentPlusIcon,
    ChevronRightIcon,
    ChevronDownIcon,
    PencilIcon,
    TrashIcon
} from '@heroicons/react/24/outline'
import { useState, useEffect } from 'react';
import { type Resource, saveNote, listNotes, renameNote, deleteNote, type Note } from '../api'
import { ContextMenu, ContextMenuItem } from './ContextMenu';

export type View = 'sources' | 'activity' | 'assistant' | 'settings'

interface SidebarProps {
    currentView: View
    onChangeView: (view: View) => void
    resources?: Resource[]
    selectedSourceId?: string | null
    onSelectSource?: (id: string | null) => void
    selectedNotePath?: string | null
    onSelectNote?: (sourceId: string, path: string) => void
    onAddSource?: () => void
}

interface ContextMenuState {
    x: number;
    y: number;
    sourceId: string;
    notePath?: string;
}

export function Sidebar({
    currentView,
    onChangeView,
    resources = [],
    selectedSourceId,
    onSelectSource,
    selectedNotePath,
    onSelectNote,
    onAddSource
}: SidebarProps) {
    const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
    const [creatingNote, setCreatingNote] = useState<string | null>(null); // sourceId where we are creating a note
    // renamingNote: { sourceId, oldPath }
    const [renamingNote, setRenamingNote] = useState<{ sourceId: string, oldPath: string } | null>(null);
    const [deleteConfirmation, setDeleteConfirmation] = useState<{ sourceId: string, notePath: string } | null>(null);
    const [expandedSources, setExpandedSources] = useState<Set<string>>(new Set());
    const [sourceNotes, setSourceNotes] = useState<Record<string, Note[]>>({});

    // Refresh notes for expanded sources periodically or when resources change
    useEffect(() => {
        resources.forEach(resource => {
            if (expandedSources.has(resource.id)) {
                loadNotes(resource.id);
            }
        });
    }, [expandedSources]);

    const loadNotes = async (sourceId: string) => {
        try {
            const notes = await listNotes(sourceId);
            setSourceNotes(prev => ({
                ...prev,
                [sourceId]: notes
            }));
        } catch (error) {
            console.error(`Failed to load notes for source ${sourceId}`, error);
        }
    };

    const toggleSourceExpansion = async (e: React.MouseEvent, sourceId: string) => {
        e.stopPropagation();
        const isExpanded = expandedSources.has(sourceId);
        const newExpanded = new Set(expandedSources);
        if (isExpanded) {
            newExpanded.delete(sourceId);
        } else {
            newExpanded.add(sourceId);
            loadNotes(sourceId);
        }
        setExpandedSources(newExpanded);
    };


    const handleSourceClick = (id: string) => {
        onChangeView('sources')
        onSelectSource?.(id)
    }

    const handleContextMenu = (e: React.MouseEvent, sourceId: string, notePath?: string) => {
        e.preventDefault();
        e.stopPropagation();
        e.nativeEvent.stopImmediatePropagation();
        setContextMenu({
            x: e.clientX,
            y: e.clientY,
            sourceId,
            notePath
        });
    };

    const startCreatingNote = (sourceId: string) => {
        setCreatingNote(sourceId);
        if (!expandedSources.has(sourceId)) {
            setExpandedSources(prev => new Set(prev).add(sourceId));
            loadNotes(sourceId);
        }
    };

    const handleCreateNoteSubmit = async (sourceId: string, filename: string) => {
        if (!filename.trim()) {
            setCreatingNote(null);
            return;
        }

        if (!filename.endsWith('.md')) {
            filename += '.md';
        }

        try {
            await saveNote(sourceId, filename, "# " + filename.replace('.md', '') + "\n\nStart writing...");
            console.log(`Created markdown: ${filename}`);
            await loadNotes(sourceId);
        } catch (err) {
            console.error("Failed to create note:", err);
            alert("Failed to create note.");
        } finally {
            setCreatingNote(null);
        }
    };

    const handleRenameNoteTrigger = () => {
        if (!contextMenu?.notePath) return;
        setRenamingNote({
            sourceId: contextMenu.sourceId,
            oldPath: contextMenu.notePath
        });
        setContextMenu(null);
    };

    const handleRenameSubmit = async (newName: string) => {
        if (!renamingNote || !newName.trim()) {
            setRenamingNote(null);
            return;
        }

        if (!newName.endsWith('.md')) {
            newName += '.md';
        }

        if (newName === renamingNote.oldPath) {
            setRenamingNote(null);
            return;
        }

        try {
            await renameNote(renamingNote.sourceId, renamingNote.oldPath, newName);
            await loadNotes(renamingNote.sourceId);
        } catch (err) {
            console.error("Failed to rename note:", err);
            alert("Failed to rename note.");
        } finally {
            setRenamingNote(null);
        }
    };

    const handleDeleteNote = () => {
        if (!contextMenu?.notePath) return;
        const { sourceId, notePath } = contextMenu;
        setContextMenu(null);
        setDeleteConfirmation({ sourceId, notePath });
    };

    const handleConfirmDelete = async () => {
        if (!deleteConfirmation) return;
        const { sourceId, notePath } = deleteConfirmation;

        try {
            await deleteNote(sourceId, notePath);
            await loadNotes(sourceId);

            // Deselect if currently selected
            if (selectedNotePath === notePath && selectedSourceId === sourceId) {
                onSelectNote?.(sourceId, '');
            }
        } catch (err) {
            console.error("Failed to delete note:", err);
            alert(`Failed to delete note: ${err instanceof Error ? err.message : String(err)}`);
        } finally {
            setDeleteConfirmation(null);
        }
    };

    const handleAddMarkdown = () => {
        if (!contextMenu) return;
        const { sourceId } = contextMenu;
        setContextMenu(null);
        startCreatingNote(sourceId);
    };

    const handleHeaderAddMarkdown = () => {
        if (!selectedSourceId) {
            alert("Please select a source first to add a markdown note.");
            return;
        }
        startCreatingNote(selectedSourceId);
    };

    const handleNoteClick = (e: React.MouseEvent, sourceId: string, notePath: string) => {
        e.stopPropagation();
        onChangeView('sources');
        if (selectedSourceId !== sourceId) {
            onSelectSource?.(sourceId);
        }
        onSelectNote?.(sourceId, notePath);
    };

    return (
        <div className="sidebar">
            <div className="sidebar-section">


                <div className="sidebar-tree">
                    <div className="tree-header" style={{
                        padding: '4px 16px',
                        display: 'flex',
                        justifyContent: 'space-between',
                        alignItems: 'center',
                        marginTop: '8px',
                        marginBottom: '4px',

                    }}>
                        <span style={{
                            fontSize: '11px',
                            fontWeight: '700',
                            color: 'var(--text-secondary)'
                        }}>SOURCES</span>

                        <div style={{ display: 'flex', gap: '4px' }}>
                            <button
                                className="icon-button"
                                onClick={(e) => {
                                    e.stopPropagation();
                                    handleHeaderAddMarkdown();
                                }}
                                title="Add Doc"
                                style={{
                                    background: 'transparent',
                                    border: 'none',
                                    color: 'var(--text-secondary)',
                                    padding: '2px',
                                    cursor: 'pointer',
                                    display: 'flex',
                                    alignItems: 'center',
                                    justifyContent: 'center',
                                    borderRadius: '4px'
                                }}
                            >
                                <DocumentPlusIcon style={{ width: '18px', height: '18px' }} />
                            </button>
                            <button
                                className="icon-button"
                                onClick={(e) => {
                                    e.stopPropagation();
                                    onAddSource?.();
                                }}
                                title="Add Source"
                                style={{
                                    background: 'transparent',
                                    border: 'none',
                                    color: 'var(--text-secondary)',
                                    padding: '2px',
                                    cursor: 'pointer',
                                    display: 'flex',
                                    alignItems: 'center',
                                    justifyContent: 'center',
                                    borderRadius: '4px'
                                }}
                            >
                                <FolderPlusIcon style={{ width: '18px', height: '18px' }} />
                            </button>
                        </div>
                    </div>

                    {resources.map(resource => (
                        <div
                            key={resource.id}
                            onContextMenu={(e) => handleContextMenu(e, resource.id)}
                            style={{ cursor: 'context-menu' }}
                        >
                            <div
                                className={`sidebar-item ${selectedSourceId === resource.id && currentView === 'sources' && !selectedNotePath ? 'active' : ''}`}
                                onClick={() => handleSourceClick(resource.id)}
                                style={{
                                    paddingLeft: '8px',
                                    width: '100%',
                                    display: 'flex',
                                    alignItems: 'center',
                                    gap: '4px'
                                }}
                            >
                                <button
                                    onClick={(e) => toggleSourceExpansion(e, resource.id)}
                                    style={{
                                        background: 'transparent',
                                        border: 'none',
                                        color: 'var(--text-secondary)',
                                        padding: '2px',
                                        cursor: 'pointer',
                                        display: 'flex',
                                        alignItems: 'center',
                                    }}
                                >
                                    {expandedSources.has(resource.id) ? (
                                        <ChevronDownIcon style={{ width: '12px', height: '12px' }} />
                                    ) : (
                                        <ChevronRightIcon style={{ width: '12px', height: '12px' }} />
                                    )}
                                </button>

                                <FolderIcon className="sidebar-icon" style={{ width: '14px', height: '14px' }} />
                                <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1 }}>
                                    {resource.name}
                                </span>
                            </div>

                            {expandedSources.has(resource.id) && (
                                <>
                                    {creatingNote === resource.id && (
                                        <div
                                            className="sidebar-item note-item"
                                            style={{
                                                paddingLeft: '32px',
                                                fontSize: '0.85rem',
                                                width: '100%',
                                                display: 'flex',
                                                alignItems: 'center',
                                                gap: '6px'
                                            }}
                                        >
                                            <div style={{
                                                fontSize: '10px',
                                                fontWeight: 'bold',
                                                color: '#60A5FA',
                                                border: '1px solid #60A5FA',
                                                borderRadius: '2px',
                                                width: '14px',
                                                height: '14px',
                                                display: 'flex',
                                                alignItems: 'center',
                                                justifyContent: 'center',
                                                lineHeight: 1
                                            }}>M</div>
                                            <input
                                                autoFocus
                                                type="text"
                                                defaultValue="New Note.md"
                                                style={{
                                                    background: 'var(--bg-app)',
                                                    border: '1px solid var(--border-color)',
                                                    borderRadius: '2px',
                                                    color: 'var(--text-primary)',
                                                    fontSize: 'inherit',
                                                    width: '100%',
                                                    outline: 'none',
                                                    padding: '1px 4px'
                                                }}
                                                onKeyDown={(e) => {
                                                    if (e.key === 'Enter') {
                                                        handleCreateNoteSubmit(resource.id, e.currentTarget.value);
                                                    } else if (e.key === 'Escape') {
                                                        setCreatingNote(null);
                                                    }
                                                    e.stopPropagation();
                                                }}
                                                onBlur={() => {
                                                    setCreatingNote(null);
                                                }}
                                                onClick={(e) => e.stopPropagation()}
                                            />
                                        </div>
                                    )}
                                    {sourceNotes[resource.id]?.map((note) => (
                                        renamingNote?.sourceId === resource.id && renamingNote.oldPath === note.path ? (
                                            <div
                                                key={note.path}
                                                className="sidebar-item note-item"
                                                style={{
                                                    paddingLeft: '32px',
                                                    fontSize: '0.85rem',
                                                    width: '100%',
                                                    display: 'flex',
                                                    alignItems: 'center',
                                                    gap: '6px'
                                                }}
                                                onClick={(e) => e.stopPropagation()}
                                            >
                                                <div style={{
                                                    fontSize: '10px',
                                                    fontWeight: 'bold',
                                                    color: '#60A5FA',
                                                    border: '1px solid #60A5FA',
                                                    borderRadius: '2px',
                                                    width: '14px',
                                                    height: '14px',
                                                    display: 'flex',
                                                    alignItems: 'center',
                                                    justifyContent: 'center',
                                                    lineHeight: 1
                                                }}>M</div>
                                                <input
                                                    autoFocus
                                                    type="text"
                                                    defaultValue={note.name}
                                                    style={{
                                                        background: 'var(--bg-app)',
                                                        border: '1px solid var(--border-color)',
                                                        borderRadius: '2px',
                                                        color: 'var(--text-primary)',
                                                        fontSize: 'inherit',
                                                        width: '100%',
                                                        outline: 'none',
                                                        padding: '1px 4px'
                                                    }}
                                                    onKeyDown={(e) => {
                                                        if (e.key === 'Enter') {
                                                            handleRenameSubmit(e.currentTarget.value);
                                                        } else if (e.key === 'Escape') {
                                                            setRenamingNote(null);
                                                        }
                                                        e.stopPropagation();
                                                    }}
                                                    onBlur={() => {
                                                        setRenamingNote(null);
                                                    }}
                                                    onClick={(e) => e.stopPropagation()}
                                                />
                                            </div>
                                        ) : (
                                            <button
                                                key={note.path}
                                                className={`sidebar-item note-item ${selectedNotePath === note.path && selectedSourceId === resource.id ? 'active' : ''}`}
                                                onClick={(e) => handleNoteClick(e, resource.id, note.path)}
                                                onContextMenu={(e) => handleContextMenu(e, resource.id, note.path)}
                                                style={{
                                                    paddingLeft: '32px',
                                                    fontSize: '0.85rem',
                                                    width: '100%',
                                                    display: 'flex',
                                                    alignItems: 'center',
                                                    gap: '6px',
                                                    color: selectedNotePath === note.path && selectedSourceId === resource.id ? 'var(--text-active)' : 'var(--text-secondary)',
                                                    backgroundColor: selectedNotePath === note.path && selectedSourceId === resource.id ? 'var(--bg-active)' : 'transparent',
                                                    opacity: 0.9
                                                }}
                                            >
                                                <div style={{
                                                    fontSize: '10px',
                                                    fontWeight: 'bold',
                                                    color: '#60A5FA',
                                                    border: '1px solid #60A5FA',
                                                    borderRadius: '2px',
                                                    width: '14px',
                                                    height: '14px',
                                                    display: 'flex',
                                                    alignItems: 'center',
                                                    justifyContent: 'center',
                                                    lineHeight: 1
                                                }}>M</div>
                                                <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                                                    {note.name}
                                                </span>
                                            </button>
                                        )
                                    ))}
                                </>
                            )}
                        </div>
                    ))}
                </div>
            </div>

            <div className="sidebar-spacer" />

            <div className="sidebar-section">
                <div className="sidebar-section-header">TOOLS</div>
                <button
                    className={`sidebar-item ${currentView === 'assistant' ? 'active' : ''}`}
                    onClick={() => onChangeView('assistant')}
                >
                    <ChatBubbleLeftRightIcon className="sidebar-icon" />
                    <span>Assistant</span>
                </button>
                <button
                    className={`sidebar-item ${currentView === 'activity' ? 'active' : ''}`}
                    onClick={() => onChangeView('activity')}
                >
                    <ClockIcon className="sidebar-icon" />
                    <span>Activity</span>
                </button>
            </div>

            <div className="sidebar-section">
                <button
                    className={`sidebar-item ${currentView === 'settings' ? 'active' : ''}`}
                    onClick={() => onChangeView('settings')}
                >
                    <Cog6ToothIcon className="sidebar-icon" />
                    <span>Settings</span>
                </button>
            </div>

            {contextMenu && (
                <ContextMenu
                    x={contextMenu.x}
                    y={contextMenu.y}
                    onClose={() => setContextMenu(null)}
                >
                    <ContextMenuItem
                        label="Add Doc"
                        icon={<DocumentPlusIcon style={{ width: '14px', height: '14px' }} />}
                        onClick={handleAddMarkdown}
                    />
                    {contextMenu.notePath && (
                        <>
                            <ContextMenuItem
                                label="Rename"
                                icon={<PencilIcon style={{ width: '14px', height: '14px' }} />}
                                onClick={handleRenameNoteTrigger}
                            />
                            <ContextMenuItem
                                label="Delete"
                                icon={<TrashIcon style={{ width: '14px', height: '14px' }} />}
                                onClick={handleDeleteNote}
                                danger={true}
                            />
                        </>
                    )}
                </ContextMenu>
            )}

            {/* Delete Confirmation Modal */}
            {deleteConfirmation && (
                <div
                    style={{
                        position: 'fixed',
                        top: 0,
                        left: 0,
                        right: 0,
                        bottom: 0,
                        backgroundColor: 'rgba(0, 0, 0, 0.5)',
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'center',
                        zIndex: 9999,
                        pointerEvents: 'auto'
                    }}
                    onClick={(e) => {
                        // Close on backdrop click
                        if (e.target === e.currentTarget) {
                            setDeleteConfirmation(null);
                        }
                    }}
                >
                    <div
                        style={{
                            backgroundColor: 'var(--bg-content)',
                            border: '1px solid var(--border-color)',
                            borderRadius: '8px',
                            padding: '24px',
                            width: '320px',
                            boxShadow: '0 4px 12px rgba(0, 0, 0, 0.3)',
                            display: 'flex',
                            flexDirection: 'column',
                            gap: '16px',
                            pointerEvents: 'auto'
                        }}
                        onClick={(e) => e.stopPropagation()}
                    >
                        <h3 style={{ margin: 0, fontSize: '1.1rem', fontWeight: 600, color: 'var(--text-primary)' }}>Delete Note?</h3>
                        <p style={{ margin: 0, fontSize: '0.9rem', color: 'var(--text-secondary)' }}>
                            Are you sure you want to delete <strong>{deleteConfirmation.notePath}</strong>? This action cannot be undone.
                        </p>
                        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '12px', marginTop: '8px' }}>
                            <button
                                onMouseDown={(e) => {
                                    e.preventDefault();
                                    e.stopPropagation();
                                    setDeleteConfirmation(null);
                                }}
                                style={{
                                    padding: '6px 12px',
                                    borderRadius: '4px',
                                    border: '1px solid var(--border-color)',
                                    background: 'transparent',
                                    color: 'var(--text-primary)',
                                    cursor: 'pointer',
                                    fontSize: '0.85rem',
                                    pointerEvents: 'auto'
                                }}
                            >
                                Cancel
                            </button>
                            <button
                                onMouseDown={(e) => {
                                    e.preventDefault();
                                    e.stopPropagation();
                                    handleConfirmDelete();
                                }}
                                style={{
                                    padding: '6px 12px',
                                    borderRadius: '4px',
                                    border: 'none',
                                    background: 'var(--error, #ef4444)',
                                    color: 'white',
                                    cursor: 'pointer',
                                    fontSize: '0.85rem',
                                    fontWeight: 600,
                                    pointerEvents: 'auto'
                                }}
                            >
                                Delete
                            </button>
                        </div>
                    </div>
                </div>
            )}
        </div>
    )
}
