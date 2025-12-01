
import { useEffect, useState } from 'react'
import { getAppStatus, type AppStatusResponse } from '../api'

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
    const [systemStatus, setSystemStatus] = useState<AppStatusResponse | null>(null)

    useEffect(() => {
        const fetchStatus = () => {
            getAppStatus().then(setSystemStatus).catch(console.error)
        }

        fetchStatus()
        const interval = setInterval(fetchStatus, 2000)
        return () => clearInterval(interval)
    }, [])

    return (
        <div className="view">
            <section className="section" style={{ marginBottom: '1.25rem' }}>
                <h3>System Status</h3>
                <div className="status-grid">
                    <div className="status-item">
                        <span className="label">Backend Status</span>
                        <span className={`value ${systemStatus?.status === 'ready' ? 'success' : systemStatus?.status === 'error' ? 'error' : 'warning'}`}>
                            {systemStatus?.status?.toUpperCase() || 'CONNECTING...'}
                        </span>
                    </div>
                    <div className="status-item">
                        <span className="label">Embedding Model</span>
                        <span className="value">
                            {systemStatus?.status === 'initializing'
                                ? (systemStatus.progress || 'Initializing...')
                                : (systemStatus?.status === 'ready' ? 'Loaded' : 'Unknown')}
                        </span>
                    </div>
                    <div className="status-item">
                        <span className="label">Active Jobs</span>
                        <span className="value">
                            {jobs.filter(j => j.status === 'running' || j.status === 'pending').length}
                        </span>
                    </div>
                </div>
            </section>

            <section className="section">
                <h3>Job History</h3>
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
