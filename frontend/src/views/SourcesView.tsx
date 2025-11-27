import { ResourceManager } from '../ResourceManager'
import type { Resource } from '../api'

interface SourcesViewProps {
    onIndexResource: (resource: Resource) => void
    indexingResourceId: string | null
    indexingProgress: string | null
    onCancelJob: () => void
    onViewProfile: (sourceId: string) => void
    resourcesVersion: number
}

export function SourcesView({
    onIndexResource,
    indexingResourceId,
    indexingProgress,
    onCancelJob,
    onViewProfile,
    resourcesVersion,
}: SourcesViewProps) {
    return (
        <div className="view">
            <section className="section">
                <ResourceManager
                    onIndexResource={onIndexResource}
                    indexingResourceId={indexingResourceId}
                    indexingProgress={indexingProgress}
                    onCancelJob={onCancelJob}
                    onViewProfile={onViewProfile}
                    refreshKey={resourcesVersion}
                />
            </section>
        </div>
    )
}
