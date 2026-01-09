
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
        <div className="p-6">
            <section className="p-4 px-5 bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-md shadow-none mb-5">
                <h3 className="text-sm font-semibold text-[var(--text-active)] mb-4 pb-2 border-b border-[var(--border-color)]">System Status</h3>
                <div className="grid grid-cols-3 gap-4">
                    <div className="flex flex-col gap-1">
                        <span className="text-[11px] font-semibold text-[var(--text-secondary)] uppercase tracking-wider">Backend Status</span>
                        <span className={`text-sm ${systemStatus?.status === 'ready' ? 'text-green-400' : systemStatus?.status === 'error' ? 'text-red-400' : 'text-amber-400'}`}>
                            {systemStatus?.status?.toUpperCase() || 'CONNECTING...'}
                        </span>
                    </div>
                    <div className="flex flex-col gap-1">
                        <span className="text-[11px] font-semibold text-[var(--text-secondary)] uppercase tracking-wider">Embedding Model</span>
                        <span className="text-sm text-[var(--text-primary)]">
                            {systemStatus?.status === 'initializing'
                                ? (systemStatus.progress || 'Initializing...')
                                : (systemStatus?.status === 'ready' ? 'Loaded' : 'Unknown')}
                        </span>
                    </div>
                    <div className="flex flex-col gap-1">
                        <span className="text-[11px] font-semibold text-[var(--text-secondary)] uppercase tracking-wider">Active Jobs</span>
                        <span className="text-sm text-[var(--text-primary)]">
                            {jobs.filter(j => j.status === 'running' || j.status === 'pending').length}
                        </span>
                    </div>
                </div>
            </section>

            <section className="p-4 px-5 bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-md shadow-none">
                <h3 className="text-sm font-semibold text-[var(--text-active)] mb-4 pb-2 border-b border-[var(--border-color)]">Job History</h3>
                {jobs.length === 0 ? (
                    <div className="p-10 text-center text-[var(--text-secondary)] text-sm">No activity yet. Add a source and click "Index now" to get started!</div>
                ) : (
                    <div className="border border-[var(--border-color)] rounded-md overflow-hidden">
                        <div className="grid grid-cols-[2fr_1.5fr_0.8fr_0.8fr_1fr_1fr] px-4 py-2.5 bg-[var(--bg-sidebar)] border-b border-[var(--border-color)] text-[11px] font-semibold text-[var(--text-secondary)] uppercase tracking-wider">
                            <span>Source</span>
                            <span>Status</span>
                            <span>Files</span>
                            <span>Chunks</span>
                            <span>Started</span>
                            <span>Finished</span>
                        </div>
                        {jobs.map((job) => (
                            <div key={job.id} className="grid grid-cols-[2fr_1.5fr_0.8fr_0.8fr_1fr_1fr] px-4 py-2.5 border-b border-[var(--border-color)] last:border-b-0 text-xs text-[var(--text-primary)] items-center hover:bg-[var(--item-hover)] transition-colors">
                                <span className="flex items-center gap-2">
                                    {job.sourceName}
                                    {job.sourceType && <span className="text-[9px] bg-[#333] px-1.5 py-0.5 rounded-[3px] text-[var(--text-secondary)] border border-[#444] capitalize">{job.sourceType}</span>}
                                </span>
                                <span>
                                    <span className={`inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-medium ${
                                        job.status === 'pending' ? 'bg-amber-500/15 text-amber-400' :
                                        job.status === 'running' ? 'bg-blue-500/15 text-blue-400' :
                                        job.status === 'completed' ? 'bg-green-500/15 text-green-400' :
                                        'bg-red-500/15 text-red-400'
                                    }`}>
                                        {job.status === 'pending' && '⏳ Pending'}
                                        {job.status === 'running' && '● Running'}
                                        {job.status === 'completed' && '✓ Completed'}
                                        {job.status === 'error' && '⚠ Error'}
                                    </span>
                                    {job.error && <span className="block text-[10px] text-red-400 mt-1 max-w-[200px] overflow-hidden text-ellipsis whitespace-nowrap" title={job.error}>{job.error}</span>}
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
