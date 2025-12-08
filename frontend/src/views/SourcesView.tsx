import type { Resource, IndexMode } from '../api'

interface SourcesViewProps {
    onIndexResource?: (resource: Resource, mode?: IndexMode) => void
    indexingResourceId?: string | null
    indexingProgress?: string | null
    onCancelJob?: () => void
    onViewProfile?: (sourceId: string) => void
    resourcesVersion?: number
}

// Since listing is now in Sidebar, this view shows when NO source is selected.
// We can use it as a "Welcome / Empty State"
// eslint-disable-next-line @typescript-eslint/no-unused-vars
export function SourcesView(_props: SourcesViewProps) {
    // We skip props usage here as we don't need the table anymore
    // but kept them in interface to not break App.tsx strict typing immediately

    return (
        <div className="view" style={{
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            height: '100%',
            color: 'var(--text-secondary)'
        }}>
            <div style={{ textAlign: 'center', maxWidth: '400px' }}>
                <div style={{ fontSize: '4rem', marginBottom: '16px', opacity: 0.2 }}>🗂️</div>
                <h2 style={{ color: 'var(--text-active)', marginBottom: '12px' }}>No Source Selected</h2>
                <p style={{ marginBottom: '32px', lineHeight: '1.6' }}>
                    Select a source from the sidebar to view its details, graph, and profile.
                    <br />
                    Or add a new source to get started.
                </p>
                <button
                    className="btn-action"
                    style={{ padding: '10px 24px', fontSize: '1rem' }}
                    onClick={() => {
                        // This local modal is just for fallback if needed, 
                        // but ideally we trigger the main app modal or just show instruction
                        // For now let's just show text, or we could lift this trigger too.
                        // Actually, since App has the modal, we rely on Sidebar button.
                        // But let's add a button that triggers the App's modal? 
                        // We don't have the handler passed down here easily without changing props.
                        // Let's just instruct user to use Sidebar for now.
                        const btn = document.querySelector('.sidebar-tree button:last-child') as HTMLButtonElement;
                        btn?.click();
                    }}
                >
                    + Add New Source
                </button>
            </div>
        </div>
    )
}
