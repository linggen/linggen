import type { ReactNode } from 'react'
import { Sidebar, type View } from './Sidebar'
import type { Resource } from '../api'

// Helper function to format bytes into human-readable size
function formatSize(bytes: number): string {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

interface MainLayoutProps {
    currentView: View
    onChangeView: (view: View) => void
    statusElement: ReactNode
    children: ReactNode
    resources?: Resource[]
    selectedSourceId?: string | null
    onSelectSource?: (id: string | null) => void
    selectedNotePath?: string | null
    onSelectNote?: (sourceId: string, path: string) => void
    selectedMemoryPath?: string | null
    onSelectMemory?: (sourceId: string, path: string) => void
    onAddSource?: () => void
}

export function MainLayout({
    currentView,
    onChangeView,
    statusElement,
    children,
    resources,
    selectedSourceId,
    onSelectSource,
    selectedNotePath,
    onSelectNote,
    selectedMemoryPath,
    onSelectMemory,
    onAddSource
}: MainLayoutProps) {
    // We can add state for collapsing sidebar later if needed
    // const [isSidebarOpen, setIsSidebarOpen] = useState(true)

    return (
        <div className="main-layout">
            {/* Left Sidebar */}
            <div className="left-sidebar">
                <div className="app-brand">
                    <div className="app-logo">LG</div>
                    <div className="app-name">Linggen Architect</div>
                </div>
                <Sidebar
                    currentView={currentView}
                    onChangeView={onChangeView}
                    resources={resources}
                    selectedSourceId={selectedSourceId}
                    onSelectSource={onSelectSource}
                    selectedNotePath={selectedNotePath}
                    onSelectNote={onSelectNote}
                    selectedMemoryPath={selectedMemoryPath}
                    onSelectMemory={onSelectMemory}
                    onAddSource={onAddSource}
                />
            </div>

            <div className="content-area">
                <header className="content-header">
                    {/* Breadcrumbs or Title could go here */}
                    <div className="view-title">{currentView.charAt(0).toUpperCase() + currentView.slice(1)}</div>
                    {currentView === 'sources' && resources && resources.length > 0 && (
                        <div style={{
                            fontSize: '0.85rem',
                            color: 'var(--text-muted)',
                            display: 'flex',
                            gap: '12px',
                            alignItems: 'center',
                            marginLeft: 'auto'
                        }}>
                            <span>{resources.length} {resources.length === 1 ? 'source' : 'sources'}</span>
                            <span>•</span>
                            <span>
                                {formatSize(
                                    resources.reduce((total, r) => 
                                        total + (r.stats?.total_size_bytes || 0), 0
                                    )
                                )}
                            </span>
                        </div>
                    )}
                </header>
                <main className="content-scroll">
                    {children}
                </main>
                {statusElement && (
                    <footer className="status-bar">
                        {statusElement}
                    </footer>
                )}
            </div>

            {/* Right Sidebar placeholder - for future 'Inspector' panel */}
            {/* 
      <aside className="right-sidebar">
        <div className="sidebar-header">Inspector</div>
        ...
      </aside> 
      */}
        </div>
    )
}
