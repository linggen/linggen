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
        <div className="flex flex-col items-center justify-center h-full text-[var(--text-secondary)]">
            <div className="text-center max-w-[400px]">
                <div className="text-[4rem] mb-4 opacity-20">🗂️</div>
                <h2 className="text-[var(--text-active)] mb-3 text-2xl font-semibold">No Project Selected</h2>
                <p className="mb-8 leading-relaxed">
                    Select a project from the sidebar to view its details, graph, and profile.
                    <br />
                    Or add a new project to get started.
                </p>
                <button
                    className="btn-primary px-6 py-2.5"
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
                    + Add New Project
                </button>
            </div>
        </div>
    )
}
