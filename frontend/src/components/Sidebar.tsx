import {
    FolderIcon,
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
import {
    type Resource,
    type LibraryPack,
    saveNote,
    listNotes,
    renameNote,
    deleteNote,
    removeResource,
    type Note,
    listMemoryFiles,
    type MemoryFile,
    createPack,
    renamePack,
    deletePack,
    createLibraryFolder,
    renameLibraryFolder,
    deleteLibraryFolder,
} from '../api'
import { ContextMenu, ContextMenuItem } from './ContextMenu';

export type View = 'sources' | 'library' | 'activity' | 'assistant' | 'settings'

interface SidebarProps {
    currentView: View
    onChangeView: (view: View) => void
    resources?: Resource[]
    resourcesVersion?: number
    selectedSourceId?: string | null
    onSelectSource?: (id: string | null) => void
    selectedNotePath?: string | null
    onSelectNote?: (sourceId: string, path: string) => void
    selectedMemoryPath?: string | null
    onSelectMemory?: (sourceId: string, path: string) => void
    selectedLibraryPackId?: string | null
    onSelectLibraryPack?: (packId: string) => void
    onAddSource?: () => void
    libraryPacks?: LibraryPack[]
    libraryFolders?: string[]
    onRefresh?: () => void
}

interface ContextMenuState {
    x: number;
    y: number;
    sourceId?: string;
    notePath?: string;
    libraryPackId?: string;
    libraryFolder?: string;
}

interface DeleteSourceConfirmation {
    sourceId: string;
    sourceName: string;
}

export function Sidebar({
    currentView,
    onChangeView,
    resources = [],
    resourcesVersion = 0,
    selectedSourceId,
    onSelectSource,
    selectedNotePath,
    onSelectNote,
    selectedMemoryPath,
    onSelectMemory,
    selectedLibraryPackId,
    onSelectLibraryPack,
    onAddSource,
    libraryPacks = [],
    libraryFolders = [],
    onRefresh
}: SidebarProps) {
    const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
    const [creatingNote, setCreatingNote] = useState<string | null>(null); // sourceId where we are creating a note
    const [creatingLibraryPack, setCreatingLibraryPack] = useState<string | null>(null); // folder where we are creating a pack
    const [creatingLibraryFolder, setCreatingLibraryFolder] = useState(false);
    
    // renaming: { type: 'note' | 'source' | 'libraryPack' | 'libraryFolder', id: string, oldPath: string }
    const [renamingNote, setRenamingNote] = useState<{ sourceId: string, oldPath: string } | null>(null);
    const [renamingLibraryPack, setRenamingLibraryPack] = useState<{ id: string, oldName: string } | null>(null);
    const [renamingLibraryFolder, setRenamingLibraryFolder] = useState<{ oldName: string } | null>(null);

    const [deleteConfirmation, setDeleteConfirmation] = useState<{ sourceId: string, notePath: string } | null>(null);
    const [deleteSourceConfirmation, setDeleteSourceConfirmation] = useState<DeleteSourceConfirmation | null>(null);
    const [deleteLibraryPackConfirmation, setDeleteLibraryPackConfirmation] = useState<{ id: string, name: string } | null>(null);
    const [deleteLibraryFolderConfirmation, setDeleteLibraryFolderConfirmation] = useState<{ name: string } | null>(null);
    const [expandedSources, setExpandedSources] = useState<Set<string>>(new Set());
    const [expandedMemories, setExpandedMemories] = useState<Set<string>>(new Set());
    const [expandedLibraryFolders, setExpandedLibraryFolders] = useState<Set<string>>(new Set(['skills', 'policies']));
    const [sourceNotes, setSourceNotes] = useState<Record<string, Note[]>>({});
    const [sourceMemories, setSourceMemories] = useState<Record<string, MemoryFile[]>>({});

    // Collapsible sections state
    const [isProjectsCollapsed, setIsProjectsCollapsed] = useState(false);
    const [isLibraryCollapsed, setIsLibraryCollapsed] = useState(false);

    // Group library packs by folder (also include empty folders)
    const libraryGroups = libraryPacks.reduce((acc, pack) => {
        const folder = pack.folder || 'general';
        if (!acc[folder]) acc[folder] = [];
        acc[folder].push(pack);
        return acc;
    }, {} as Record<string, LibraryPack[]>);

    for (const folder of libraryFolders) {
        if (!libraryGroups[folder]) {
            libraryGroups[folder] = [];
        }
    }

    const toggleLibraryFolder = (folder: string) => {
        const newSet = new Set(expandedLibraryFolders);
        if (newSet.has(folder)) newSet.delete(folder);
        else newSet.add(folder);
        setExpandedLibraryFolders(newSet);
    };

    // Refresh notes for expanded sources periodically or when resources/version change
    useEffect(() => {
        resources.forEach(resource => {
            if (expandedSources.has(resource.id)) {
                loadNotes(resource.id);
                loadMemories(resource.id);
            }
        });
    }, [expandedSources, resources, resourcesVersion]);

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

    const loadMemories = async (sourceId: string) => {
        try {
            const files = await listMemoryFiles(sourceId);
            setSourceMemories(prev => ({
                ...prev,
                [sourceId]: files
            }));
            // Auto-expand memories on first load if there are files and the user hasn't toggled yet.
            if (files.length > 0) {
                setExpandedMemories(prev => {
                    if (prev.has(sourceId)) return prev;
                    const next = new Set(prev);
                    next.add(sourceId);
                    return next;
                });
            }
        } catch (error) {
            console.error(`Failed to load memories for source ${sourceId}`, error);
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
            loadMemories(sourceId);
        }
        setExpandedSources(newExpanded);
    };


    const handleSourceClick = (id: string) => {
        onChangeView('sources')
        onSelectSource?.(id)
    }

    const handleMemoryClick = (e: React.MouseEvent, sourceId: string, path: string) => {
        e.stopPropagation();
        onChangeView('sources');
        if (selectedSourceId !== sourceId) {
            onSelectSource?.(sourceId);
        }
        onSelectMemory?.(sourceId, path);
    }

    const toggleMemoriesExpansion = (e: React.MouseEvent, sourceId: string) => {
        e.stopPropagation();
        setExpandedMemories(prev => {
            const next = new Set(prev);
            if (next.has(sourceId)) {
                next.delete(sourceId);
            } else {
                next.add(sourceId);
            }
            return next;
        });
    };

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
        if (!contextMenu?.notePath || !contextMenu?.sourceId) return;
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
        if (!contextMenu?.notePath || !contextMenu?.sourceId) return;
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
        if (!contextMenu || !contextMenu.sourceId) return;
        const { sourceId } = contextMenu;
        setContextMenu(null);
        startCreatingNote(sourceId);
    };

    const handleRemoveSource = () => {
        if (!contextMenu || !contextMenu.sourceId || contextMenu.notePath) return; // Only for sources, not notes
        const { sourceId } = contextMenu;
        const resource = resources.find(r => r.id === sourceId);
        setContextMenu(null);
        
        if (resource) {
            setDeleteSourceConfirmation({
                sourceId,
                sourceName: resource.name
            });
        }
    };

    const handleConfirmRemoveSource = async () => {
        if (!deleteSourceConfirmation) return;
        const { sourceId } = deleteSourceConfirmation;

        try {
            await removeResource(sourceId);
            
            // Deselect if currently selected
            if (selectedSourceId === sourceId) {
                onSelectSource?.(null);
            }
            
            // Collapse if expanded
            if (expandedSources.has(sourceId)) {
                const newExpanded = new Set(expandedSources);
                newExpanded.delete(sourceId);
                setExpandedSources(newExpanded);
            }
            
            // Note: The parent App.tsx will re-fetch resources automatically
        } catch (err) {
            console.error("Failed to remove source:", err);
            alert(`Failed to remove source: ${err instanceof Error ? err.message : String(err)}`);
        } finally {
            setDeleteSourceConfirmation(null);
        }
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

    const handleLibraryContextMenu = (e: React.MouseEvent, packId?: string, folder?: string) => {
        e.preventDefault();
        e.stopPropagation();
        e.nativeEvent.stopImmediatePropagation();
        setContextMenu({
            x: e.clientX,
            y: e.clientY,
            libraryPackId: packId,
            libraryFolder: folder
        });
    };

    const handleCreateLibraryPackSubmit = async (folder: string, name: string) => {
        if (!name.trim()) {
            setCreatingLibraryPack(null);
            return;
        }

        try {
            await createPack(folder, name);
            onRefresh?.();
        } catch (err) {
            console.error("Failed to create library pack:", err);
            alert("Failed to create library pack.");
        } finally {
            setCreatingLibraryPack(null);
        }
    };

    const handleCreateLibraryFolderSubmit = async (name: string) => {
        if (!name.trim()) {
            setCreatingLibraryFolder(false);
            return;
        }

        try {
            await createLibraryFolder(name);
            onRefresh?.();
        } catch (err) {
            console.error("Failed to create library folder:", err);
            alert("Failed to create library folder.");
        } finally {
            setCreatingLibraryFolder(false);
        }
    };

    const handleRenameLibraryPackSubmit = async (newName: string) => {
        if (!renamingLibraryPack || !newName.trim()) {
            setRenamingLibraryPack(null);
            return;
        }

        if (newName === renamingLibraryPack.oldName) {
            setRenamingLibraryPack(null);
            return;
        }

        try {
            await renamePack(renamingLibraryPack.id, newName);
            onRefresh?.();
        } catch (err) {
            console.error("Failed to rename library pack:", err);
            alert("Failed to rename library pack.");
        } finally {
            setRenamingLibraryPack(null);
        }
    };

    const handleRenameLibraryFolderSubmit = async (newName: string) => {
        if (!renamingLibraryFolder || !newName.trim()) {
            setRenamingLibraryFolder(null);
            return;
        }

        if (newName === renamingLibraryFolder.oldName) {
            setRenamingLibraryFolder(null);
            return;
        }

        try {
            await renameLibraryFolder(renamingLibraryFolder.oldName, newName);
            onRefresh?.();
        } catch (err) {
            console.error("Failed to rename library folder:", err);
            alert("Failed to rename library folder.");
        } finally {
            setRenamingLibraryFolder(null);
        }
    };

    const handleConfirmDeleteLibraryPack = async () => {
        if (!deleteLibraryPackConfirmation) return;
        try {
            await deletePack(deleteLibraryPackConfirmation.id);
            onRefresh?.();
            if (selectedLibraryPackId === deleteLibraryPackConfirmation.id) {
                onSelectLibraryPack?.('');
            }
        } catch (err) {
            console.error("Failed to delete library pack:", err);
            alert("Failed to delete library pack.");
        } finally {
            setDeleteLibraryPackConfirmation(null);
        }
    };

    const handleConfirmDeleteLibraryFolder = async () => {
        if (!deleteLibraryFolderConfirmation) return;
        try {
            await deleteLibraryFolder(deleteLibraryFolderConfirmation.name);
            onRefresh?.();
        } catch (err) {
            console.error("Failed to delete library folder:", err);
            alert("Failed to delete library folder.");
        } finally {
            setDeleteLibraryFolderConfirmation(null);
        }
    };

    const SidebarSectionHeader = ({ 
        label, 
        isCollapsed, 
        onToggle, 
        actions 
    }: { 
        label: string, 
        isCollapsed: boolean, 
        onToggle: () => void, 
        actions?: React.ReactNode 
    }) => (
        <div className="tree-header" style={{
            padding: '4px 16px',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            marginTop: '8px',
            marginBottom: '4px',
            cursor: 'pointer'
        }} onClick={onToggle}>
            <div style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
                {isCollapsed ? (
                    <ChevronRightIcon style={{ width: '10px', height: '10px', color: 'var(--text-secondary)' }} />
                ) : (
                    <ChevronDownIcon style={{ width: '10px', height: '10px', color: 'var(--text-secondary)' }} />
                )}
                <span style={{
                    fontSize: '11px',
                    fontWeight: '700',
                    color: 'var(--text-secondary)',
                    letterSpacing: '0.05em'
                }}>{label}</span>
            </div>

            {actions && (
                <div style={{ display: 'flex', gap: '4px' }} onClick={e => e.stopPropagation()}>
                    {actions}
                </div>
            )}
        </div>
    );

    return (
        <div className="sidebar">
            <div className="sidebar-section">
                <div className="sidebar-tree">
                    <SidebarSectionHeader 
                        label="PROJECTS"
                        isCollapsed={isProjectsCollapsed}
                        onToggle={() => setIsProjectsCollapsed(!isProjectsCollapsed)}
                        actions={
                            <>
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
                                    title="Add Project"
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
                            </>
                        }
                    />

                    {!isProjectsCollapsed && resources.map(resource => (
                        <div
                            key={resource.id}
                            onContextMenu={(e) => handleContextMenu(e, resource.id)}
                            style={{ cursor: 'context-menu' }}
                        >
                            <div
                                className={`sidebar-item ${selectedSourceId === resource.id && currentView === 'sources' && !selectedNotePath && !selectedMemoryPath ? 'active' : ''}`}
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

                                    {/* Memories */}
                                    {sourceMemories[resource.id]?.length ? (
                                        <>
                                            <button
                                                className="sidebar-item note-item"
                                                onClick={(e) => toggleMemoriesExpansion(e, resource.id)}
                                                style={{
                                                    paddingLeft: '32px',
                                                    marginTop: '6px',
                                                    marginBottom: '4px',
                                                    fontSize: '0.7rem',
                                                    width: '100%',
                                                    display: 'flex',
                                                    alignItems: 'center',
                                                    gap: '6px',
                                                    color: 'var(--text-muted)',
                                                    background: 'transparent',
                                                    textTransform: 'uppercase',
                                                    letterSpacing: '0.06em',
                                                    opacity: 0.9,
                                                }}
                                                title={expandedMemories.has(resource.id) ? 'Collapse memories' : 'Expand memories'}
                                            >
                                                {expandedMemories.has(resource.id) ? (
                                                    <ChevronDownIcon className="sidebar-icon" style={{ width: '14px', height: '14px' }} />
                                                ) : (
                                                    <ChevronRightIcon className="sidebar-icon" style={{ width: '14px', height: '14px' }} />
                                                )}
                                                <span style={{ flex: 1, textAlign: 'left' }}>Memories</span>
                                                <span style={{ fontSize: '0.65rem', opacity: 0.8 }}>
                                                    {sourceMemories[resource.id]?.length || 0}
                                                </span>
                                            </button>

                                            {expandedMemories.has(resource.id) &&
                                                sourceMemories[resource.id]?.map((mem) => (
                                                    <button
                                                        key={mem.path}
                                                        className={`sidebar-item note-item ${selectedMemoryPath === mem.path && selectedSourceId === resource.id ? 'active' : ''}`}
                                                        onClick={(e) => handleMemoryClick(e, resource.id, mem.path)}
                                                        style={{
                                                            paddingLeft: '48px',
                                                            fontSize: '0.85rem',
                                                            width: '100%',
                                                            display: 'flex',
                                                            alignItems: 'center',
                                                            gap: '6px',
                                                            color:
                                                                selectedMemoryPath === mem.path && selectedSourceId === resource.id
                                                                    ? 'var(--text-active)'
                                                                    : 'var(--text-secondary)',
                                                            backgroundColor:
                                                                selectedMemoryPath === mem.path && selectedSourceId === resource.id
                                                                    ? 'var(--bg-active)'
                                                                    : 'transparent',
                                                            opacity: 0.9,
                                                        }}
                                                    >
                                                        <div
                                                            style={{
                                                                fontSize: '10px',
                                                                fontWeight: 'bold',
                                                                color: '#A78BFA',
                                                                border: '1px solid #A78BFA',
                                                                borderRadius: '2px',
                                                                width: '14px',
                                                                height: '14px',
                                                                display: 'flex',
                                                                alignItems: 'center',
                                                                justifyContent: 'center',
                                                                lineHeight: 1,
                                                            }}
                                                        >
                                                            M
                                                        </div>
                                                        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                                                            {mem.name}
                                                        </span>
                                                    </button>
                                                ))}
                                        </>
                                    ) : null}
                                </>
                            )}
                        </div>
                    ))}
                </div>

                <div className="sidebar-tree" style={{ marginTop: '16px' }}>
                    <SidebarSectionHeader 
                        label="LIBRARY"
                        isCollapsed={isLibraryCollapsed}
                        onToggle={() => {
                            setIsLibraryCollapsed(!isLibraryCollapsed);
                            onChangeView('library');
                        }}
                        actions={
                            <>
                                <button
                                    className="icon-button"
                                    onClick={(e) => {
                                        e.stopPropagation();
                                        setCreatingLibraryPack('general');
                                        if (isLibraryCollapsed) setIsLibraryCollapsed(false);
                                    }}
                                    title="Add Library File"
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
                                        setCreatingLibraryFolder(true);
                                        if (isLibraryCollapsed) setIsLibraryCollapsed(false);
                                    }}
                                    title="Add Library Folder"
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
                            </>
                        }
                    />

                    {!isLibraryCollapsed && (
                        <div style={{ padding: '0 8px' }}>
                            {creatingLibraryFolder && (
                                <div
                                    className="sidebar-item note-item"
                                    style={{
                                        paddingLeft: '16px',
                                        fontSize: '0.85rem',
                                        width: '100%',
                                        display: 'flex',
                                        alignItems: 'center',
                                        gap: '6px'
                                    }}
                                >
                                    <FolderIcon style={{ width: '14px', height: '14px', color: 'var(--text-muted)' }} />
                                    <input
                                        autoFocus
                                        type="text"
                                        defaultValue="new-folder"
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
                                                handleCreateLibraryFolderSubmit(e.currentTarget.value);
                                            } else if (e.key === 'Escape') {
                                                setCreatingLibraryFolder(false);
                                            }
                                        }}
                                        onBlur={() => setCreatingLibraryFolder(false)}
                                    />
                                </div>
                            )}
                            {Object.entries(libraryGroups).map(([folder, packs]) => {
                                const isRenamingFolder = folder && renamingLibraryFolder?.oldName === folder;
                                return (
                                    <div key={folder}>
                                        {isRenamingFolder ? (
                                            <div
                                                className="sidebar-item note-item"
                                                style={{
                                                    paddingLeft: '16px',
                                                    fontSize: '0.85rem',
                                                    width: '100%',
                                                    display: 'flex',
                                                    alignItems: 'center',
                                                    gap: '6px'
                                                }}
                                            >
                                                <FolderIcon style={{ width: '14px', height: '14px', color: 'var(--text-muted)' }} />
                                                <input
                                                    autoFocus
                                                    type="text"
                                                    defaultValue={folder}
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
                                                            handleRenameLibraryFolderSubmit(e.currentTarget.value);
                                                        } else if (e.key === 'Escape') {
                                                            setRenamingLibraryFolder(null);
                                                        }
                                                    }}
                                                    onBlur={() => setRenamingLibraryFolder(null)}
                                                />
                                            </div>
                                        ) : (
                                            <button
                                                className="sidebar-item note-item"
                                                onClick={() => toggleLibraryFolder(folder)}
                                                onContextMenu={(e) => handleLibraryContextMenu(e, undefined, folder)}
                                                style={{ 
                                                    paddingLeft: '16px',
                                                    fontSize: '0.7rem',
                                                    width: '100%',
                                                    display: 'flex',
                                                    alignItems: 'center',
                                                    gap: '6px',
                                                    color: 'var(--text-muted)',
                                                    background: 'transparent',
                                                    textTransform: 'uppercase',
                                                    letterSpacing: '0.06em',
                                                    opacity: 0.9,
                                                }}
                                            >
                                                {expandedLibraryFolders.has(folder) ? (
                                                    <ChevronDownIcon style={{ width: '12px', height: '12px' }} />
                                                ) : (
                                                    <ChevronRightIcon style={{ width: '12px', height: '12px' }} />
                                                )}
                                                <span style={{ flex: 1, textAlign: 'left' }}>{folder}</span>
                                                <span style={{ fontSize: '0.65rem', opacity: 0.8 }}>
                                                    {packs.length}
                                                </span>
                                            </button>
                                        )}
                                        {expandedLibraryFolders.has(folder) && (
                                            <div style={{ paddingLeft: '8px' }}>
                                                {creatingLibraryPack === folder && (
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
                                                        }}>L</div>
                                                        <input
                                                            autoFocus
                                                            type="text"
                                                            defaultValue="New Pack"
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
                                                                    handleCreateLibraryPackSubmit(folder, e.currentTarget.value);
                                                                } else if (e.key === 'Escape') {
                                                                    setCreatingLibraryPack(null);
                                                                }
                                                            }}
                                                            onBlur={() => setCreatingLibraryPack(null)}
                                                        />
                                                    </div>
                                                )}
                                                {packs.map(pack => {
                                                    const isRenaming = pack.id && renamingLibraryPack?.id === pack.id;
                                                    return isRenaming ? (
                                                        <div
                                                            key={pack.id || 'renaming'}
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
                                                                color: pack.color || '#A78BFA',
                                                                border: `1px solid ${pack.color || '#A78BFA'}`,
                                                                borderRadius: '2px',
                                                                width: '14px',
                                                                height: '14px',
                                                                display: 'flex',
                                                                alignItems: 'center',
                                                                justifyContent: 'center',
                                                                lineHeight: 1
                                                            }}>L</div>
                                                            <input
                                                                autoFocus
                                                                type="text"
                                                                defaultValue={pack.filename || pack.name}
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
                                                                        handleRenameLibraryPackSubmit(e.currentTarget.value);
                                                                    } else if (e.key === 'Escape') {
                                                                        setRenamingLibraryPack(null);
                                                                    }
                                                                }}
                                                                onBlur={() => setRenamingLibraryPack(null)}
                                                            />
                                                        </div>
                                                    ) : (
                                                        <button
                                                            key={pack.id}
                                                            className={`sidebar-item note-item ${currentView === 'library' && selectedLibraryPackId === pack.id ? 'active' : ''}`}
                                                            onClick={() => pack.id && onSelectLibraryPack?.(pack.id)}
                                                            onContextMenu={(e) => pack.id && handleLibraryContextMenu(e, pack.id)}
                                                            style={{
                                                                paddingLeft: '32px',
                                                                fontSize: '0.85rem',
                                                                width: '100%',
                                                                display: 'flex',
                                                                alignItems: 'center',
                                                                gap: '6px',
                                                                color: selectedLibraryPackId === pack.id ? 'var(--text-active)' : 'var(--text-secondary)',
                                                                backgroundColor: selectedLibraryPackId === pack.id ? 'var(--bg-active)' : 'transparent',
                                                                opacity: 0.9,
                                                            }}
                                                        >
                                                            <div
                                                                style={{
                                                                    fontSize: '10px',
                                                                    fontWeight: 'bold',
                                                                    color: pack.color || '#A78BFA',
                                                                    border: `1px solid ${pack.color || '#A78BFA'}`,
                                                                    borderRadius: '2px',
                                                                    width: '14px',
                                                                    height: '14px',
                                                                    display: 'flex',
                                                                    alignItems: 'center',
                                                                    justifyContent: 'center',
                                                                    lineHeight: 1,
                                                                }}
                                                            >
                                                                L
                                                            </div>
                                                            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                                                                {pack.filename || pack.name}
                                                            </span>
                                                        </button>
                                                    );
                                                })}
                                            </div>
                                        )}
                                    </div>
                                );
                            })}
                            {libraryPacks.length === 0 && !creatingLibraryFolder && (
                                <div style={{ 
                                    padding: '8px 16px', 
                                    fontSize: '11px', 
                                    color: 'var(--text-muted)',
                                    fontStyle: 'italic'
                                }}>
                                    No library packs found.
                                </div>
                            )}
                        </div>
                    )}
                </div>
            </div>

            <div className="sidebar-spacer" />

            <div className="sidebar-section">
                <div className="sidebar-section-header">TOOLS</div>
                {/* Assistant view is hidden until it's complete */}
                {/* <button
                    className={`sidebar-item ${currentView === 'assistant' ? 'active' : ''}`}
                    onClick={() => onChangeView('assistant')}
                >
                    <ChatBubbleLeftRightIcon className="sidebar-icon" />
                    <span>Assistant</span>
                </button> */}
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
                    {contextMenu.libraryPackId ? (
                        <>
                            <ContextMenuItem
                                label="Rename"
                                icon={<PencilIcon style={{ width: '14px', height: '14px' }} />}
                                onClick={() => {
                                    const pack = libraryPacks.find(p => p.id === contextMenu.libraryPackId);
                                    if (pack) {
                                        setRenamingLibraryPack({ id: pack.id, oldName: pack.filename || pack.name });
                                    }
                                    setContextMenu(null);
                                }}
                            />
                            <ContextMenuItem
                                label="Delete"
                                icon={<TrashIcon style={{ width: '14px', height: '14px' }} />}
                                onClick={() => {
                                    const pack = libraryPacks.find(p => p.id === contextMenu.libraryPackId);
                                    if (pack) {
                                        setDeleteLibraryPackConfirmation({ id: pack.id, name: pack.filename || pack.name });
                                    }
                                    setContextMenu(null);
                                }}
                                danger={true}
                            />
                        </>
                    ) : contextMenu.libraryFolder ? (
                        <>
                            <ContextMenuItem
                                label="Add Doc"
                                icon={<DocumentPlusIcon style={{ width: '14px', height: '14px' }} />}
                                onClick={() => {
                                    setCreatingLibraryPack(contextMenu.libraryFolder!);
                                    setContextMenu(null);
                                }}
                            />
                            <ContextMenuItem
                                label="Rename"
                                icon={<PencilIcon style={{ width: '14px', height: '14px' }} />}
                                onClick={() => {
                                    setRenamingLibraryFolder({ oldName: contextMenu.libraryFolder! });
                                    setContextMenu(null);
                                }}
                            />
                            <ContextMenuItem
                                label="Delete"
                                icon={<TrashIcon style={{ width: '14px', height: '14px' }} />}
                                onClick={() => {
                                    setDeleteLibraryFolderConfirmation({ name: contextMenu.libraryFolder! });
                                    setContextMenu(null);
                                }}
                                danger={true}
                            />
                        </>
                    ) : (
                        <>
                            <ContextMenuItem
                                label="Add Doc"
                                icon={<DocumentPlusIcon style={{ width: '14px', height: '14px' }} />}
                                onClick={handleAddMarkdown}
                            />
                            {contextMenu.notePath ? (
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
                            ) : (
                                <ContextMenuItem
                                    label="Remove Project"
                                    icon={<TrashIcon style={{ width: '14px', height: '14px' }} />}
                                    onClick={handleRemoveSource}
                                    danger={true}
                                />
                            )}
                        </>
                    )}
                </ContextMenu>
            )}

            {/* Delete Note Confirmation Modal */}
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

            {/* Remove Source Confirmation Modal */}
            {deleteSourceConfirmation && (
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
                            setDeleteSourceConfirmation(null);
                        }
                    }}
                >
                    <div
                        style={{
                            backgroundColor: 'var(--bg-content)',
                            border: '1px solid var(--border-color)',
                            borderRadius: '8px',
                            padding: '24px',
                            width: '400px',
                            boxShadow: '0 4px 12px rgba(0, 0, 0, 0.3)',
                            display: 'flex',
                            flexDirection: 'column',
                            gap: '16px',
                            pointerEvents: 'auto'
                        }}
                        onClick={(e) => e.stopPropagation()}
                    >
                        <h3 style={{ margin: 0, fontSize: '1.1rem', fontWeight: 600, color: '#ef4444' }}>⚠️ Remove Source?</h3>
                        <div style={{ margin: 0, fontSize: '0.9rem', color: 'var(--text-secondary)' }}>
                            <p style={{ margin: '0 0 12px 0' }}>
                                Are you sure you want to remove <strong>{deleteSourceConfirmation.sourceName}</strong>?
                            </p>
                            <p style={{ margin: '0 0 12px 0', fontSize: '0.85rem' }}>
                                This will permanently delete:
                            </p>
                            <ul style={{ margin: '0 0 12px 20px', fontSize: '0.85rem', lineHeight: '1.6' }}>
                                <li>All indexed files and chunks</li>
                                <li>All vector embeddings (LanceDB)</li>
                                <li>All metadata (redb)</li>
                                <li>All notes and documents</li>
                                <li>Graph cache</li>
                            </ul>
                            <p style={{ margin: 0, fontSize: '0.85rem', color: '#ef4444', fontWeight: 600 }}>
                                ⚠️ This action cannot be undone!
                            </p>
                        </div>
                        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '12px', marginTop: '8px' }}>
                            <button
                                onMouseDown={(e) => {
                                    e.preventDefault();
                                    e.stopPropagation();
                                    setDeleteSourceConfirmation(null);
                                }}
                                style={{
                                    padding: '8px 16px',
                                    borderRadius: '4px',
                                    border: '1px solid var(--border-color)',
                                    background: 'transparent',
                                    color: 'var(--text-primary)',
                                    cursor: 'pointer',
                                    fontSize: '0.9rem',
                                    pointerEvents: 'auto'
                                }}
                            >
                                Cancel
                            </button>
                            <button
                                onMouseDown={(e) => {
                                    e.preventDefault();
                                    e.stopPropagation();
                                    handleConfirmRemoveSource();
                                }}
                                style={{
                                    padding: '8px 16px',
                                    borderRadius: '4px',
                                    border: 'none',
                                    background: '#ef4444',
                                    color: 'white',
                                    cursor: 'pointer',
                                    fontSize: '0.9rem',
                                    fontWeight: 600,
                                    pointerEvents: 'auto'
                                }}
                            >
                                Remove Source
                            </button>
                        </div>
                    </div>
                </div>
            )}
            {/* Delete Library Pack Confirmation Modal */}
            {deleteLibraryPackConfirmation && (
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
                        if (e.target === e.currentTarget) {
                            setDeleteLibraryPackConfirmation(null);
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
                        <h3 style={{ margin: 0, fontSize: '1.1rem', fontWeight: 600, color: 'var(--text-primary)' }}>Delete Library Pack?</h3>
                        <p style={{ margin: 0, fontSize: '0.9rem', color: 'var(--text-secondary)' }}>
                            Are you sure you want to delete <strong>{deleteLibraryPackConfirmation.name}</strong>? This action cannot be undone.
                        </p>
                        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '12px', marginTop: '8px' }}>
                            <button
                                onClick={() => setDeleteLibraryPackConfirmation(null)}
                                style={{
                                    padding: '6px 12px',
                                    borderRadius: '4px',
                                    border: '1px solid var(--border-color)',
                                    background: 'transparent',
                                    color: 'var(--text-primary)',
                                    cursor: 'pointer',
                                    fontSize: '0.85rem'
                                }}
                            >
                                Cancel
                            </button>
                            <button
                                onClick={handleConfirmDeleteLibraryPack}
                                style={{
                                    padding: '6px 12px',
                                    borderRadius: '4px',
                                    border: 'none',
                                    background: 'var(--error, #ef4444)',
                                    color: 'white',
                                    cursor: 'pointer',
                                    fontSize: '0.85rem',
                                    fontWeight: 600
                                }}
                            >
                                Delete
                            </button>
                        </div>
                    </div>
                </div>
            )}

            {/* Delete Library Folder Confirmation Modal */}
            {deleteLibraryFolderConfirmation && (
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
                        if (e.target === e.currentTarget) {
                            setDeleteLibraryFolderConfirmation(null);
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
                        <h3 style={{ margin: 0, fontSize: '1.1rem', fontWeight: 600, color: 'var(--text-primary)' }}>Delete Library Folder?</h3>
                        <p style={{ margin: 0, fontSize: '0.9rem', color: 'var(--text-secondary)' }}>
                            Are you sure you want to delete folder <strong>{deleteLibraryFolderConfirmation.name}</strong> and all its contents? This action cannot be undone.
                        </p>
                        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '12px', marginTop: '8px' }}>
                            <button
                                onClick={() => setDeleteLibraryFolderConfirmation(null)}
                                style={{
                                    padding: '6px 12px',
                                    borderRadius: '4px',
                                    border: '1px solid var(--border-color)',
                                    background: 'transparent',
                                    color: 'var(--text-primary)',
                                    cursor: 'pointer',
                                    fontSize: '0.85rem'
                                }}
                            >
                                Cancel
                            </button>
                            <button
                                onClick={handleConfirmDeleteLibraryFolder}
                                style={{
                                    padding: '6px 12px',
                                    borderRadius: '4px',
                                    border: 'none',
                                    background: 'var(--error, #ef4444)',
                                    color: 'white',
                                    cursor: 'pointer',
                                    fontSize: '0.85rem',
                                    fontWeight: 600
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
