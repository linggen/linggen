

type JobStatus = 'pending' | 'running' | 'completed' | 'error'

interface Job {
    id: string
    sourceId?: string
    sourceName: string
    sourceType?: string
    startedAt: string
    finishedAt?: string
    status: JobStatus
    filesIndexed?: number
    chunksCreated?: number
    error?: string
}

interface ActivityViewProps {
    jobs: Job[]
}

export function ActivityView({ jobs }: ActivityViewProps) {
    return (
        <div className="view">
            <section className="section">
                <div className="view-header" style={{ marginBottom: '1rem', minHeight: 'auto' }}>
                    <h2>Activity</h2>
                </div>
                {jobs.length === 0 ? (
                    <div className="empty-state">No activity yet. Add a source and click "Index now" to get started!</div>
                ) : (
                    <div className="jobs-table">
                        <div className="jobs-table-header">
                            <span>Source</span>
                            <span>Status</span>
                            <span>Files</span>
                            <span>Chunks</span>
                            <span>Started</span>
                            <span>Finished</span>
                        </div>
                        {jobs.map((job) => (
                            <div key={job.id} className="jobs-table-row">
                                <span className="job-source">
                                    {job.sourceName}
                                    {job.sourceType && <span className="job-source-type">{job.sourceType}</span>}
                                </span>
                                <span>
                                    <span className={`status-badge job-${job.status}`}>
                                        {job.status === 'pending' && '⏳ Pending'}
                                        {job.status === 'running' && '● Running'}
                                        {job.status === 'completed' && '✓ Completed'}
                                        {job.status === 'error' && '⚠ Error'}
                                    </span>
                                    {job.error && <span className="job-error">{job.error}</span>}
                                </span>
                                <span>{job.filesIndexed ?? '—'}</span>
                                <span>{job.chunksCreated ?? '—'}</span>
                                <span>{new Date(job.startedAt).toLocaleTimeString()}</span>
                                <span>{job.finishedAt ? new Date(job.finishedAt).toLocaleTimeString() : '—'}</span>
                            </div>
                        ))}
                    </div>
                )}
            </section>
        </div>
    )
}
