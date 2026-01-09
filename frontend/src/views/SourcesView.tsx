import type { Resource, IndexMode } from '../api'

interface SourcesViewProps {
    onIndexResource?: (resource: Resource, mode?: IndexMode) => void
    indexingResourceId?: string | null
    indexingProgress?: string | null
    onCancelJob?: () => void
    onViewProfile?: (sourceId: string) => void
    resourcesVersion?: number
    onAddSource?: () => void
}

// Since listing is now in Sidebar, this view shows when NO source is selected.
// We can use it as a "Welcome / Empty State"
export function SourcesView({ onAddSource }: SourcesViewProps) {
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
                    onClick={onAddSource}
                >
                    + Add New Project
                </button>
            </div>
        </div>
    )
}
