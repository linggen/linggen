import {
    FolderIcon,
    Square3Stack3DIcon,
    ChatBubbleLeftRightIcon,
    ClockIcon,
    Cog6ToothIcon,
    PlusIcon,
    DocumentPlusIcon
} from '@heroicons/react/24/outline'
import { useState } from 'react';
import { type Resource, saveNote } from '../api'
import { ContextMenu, ContextMenuItem } from './ContextMenu';

export type View = 'sources' | 'architecture' | 'activity' | 'assistant' | 'settings'

interface SidebarProps {
    currentView: View
    onChangeView: (view: View) => void
    resources?: Resource[]
    selectedSourceId?: string | null
    onSelectSource?: (id: string | null) => void
    onAddSource?: () => void
}

interface ContextMenuState {
    x: number;
    y: number;
    sourceId: string;
}

export function Sidebar({
    currentView,
    onChangeView,
    resources = [],
    selectedSourceId,
    onSelectSource,
    onAddSource
}: SidebarProps) {
    const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);

    const handleSourceClick = (id: string) => {
        onChangeView('sources')
        onSelectSource?.(id)
    }

    const handleContextMenu = (e: React.MouseEvent, sourceId: string) => {
        e.preventDefault();
        e.stopPropagation();
        e.nativeEvent.stopImmediatePropagation();
        setContextMenu({
            x: e.clientX,
            y: e.clientY,
            sourceId
        });
    };

    const handleAddMarkdown = async () => {
        if (!contextMenu) return;
        const { sourceId } = contextMenu;
        setContextMenu(null);

        const filename = window.prompt("Enter filename (e.g., Note.md):", "New Note.md");
        if (!filename) return;

        try {
            await saveNote(sourceId, filename, "# New Note\n\nStart writing...");
            // Optionally notify success or refresh
            // Currently we don't have a way to view notes in Sidebar, but they are created.
            console.log(`Created markdown: ${filename}`);
        } catch (err) {
            console.error("Failed to create note:", err);
            alert("Failed to create note.");
        }
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
                            <PlusIcon style={{ width: '14px', height: '14px' }} />
                        </button>
                    </div>

                    {resources.map(resource => (
                        <div
                            key={resource.id}
                            onContextMenu={(e) => handleContextMenu(e, resource.id)}
                            style={{ cursor: 'context-menu' }}
                        >
                            <button
                                className={`sidebar-item ${selectedSourceId === resource.id && currentView === 'sources' ? 'active' : ''}`}
                                onClick={() => handleSourceClick(resource.id)}
                                style={{ paddingLeft: '24px', width: '100%' }}
                            >
                                <FolderIcon className="sidebar-icon" style={{ width: '14px', height: '14px' }} />
                                <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                                    {resource.name}
                                </span>
                            </button>
                        </div>
                    ))}
                </div>
            </div>

            <div className="sidebar-spacer" />

            <div className="sidebar-section">
                <div className="sidebar-section-header">TOOLS</div>
                <button
                    className={`sidebar-item ${currentView === 'architecture' ? 'active' : ''}`}
                    onClick={() => onChangeView('architecture')}
                >
                    <Square3Stack3DIcon className="sidebar-icon" />
                    <span>Architecture</span>
                </button>
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
                        label="Add Markdown"
                        icon={<DocumentPlusIcon style={{ width: '14px', height: '14px' }} />}
                        onClick={handleAddMarkdown}
                    />
                    {/* Placeholder for future actions */}
                </ContextMenu>
            )}
        </div>
    )
}
