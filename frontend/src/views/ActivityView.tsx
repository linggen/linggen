
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
        <div className="flex-1 overflow-y-auto p-6 flex flex-col gap-6 max-w-[1000px] mx-auto">
            <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-xl overflow-hidden shadow-sm">
                <div className="flex items-center gap-2.5 px-5 py-3.5 bg-[var(--item-hover)] border-b border-[var(--border-color)]">
                    <span className="text-base">🖥️</span>
                    <h3 className="m-0 text-sm font-semibold text-[var(--text-active)] border-none p-0">System Status</h3>
                </div>
                <div className="p-6 grid grid-cols-1 md:grid-cols-3 gap-6">
                    <div className="flex flex-col gap-1.5 p-4 bg-[var(--bg-app)] rounded-lg border border-[var(--border-color)]/50">
                        <span className="text-[10px] font-bold text-[var(--text-muted)] uppercase tracking-widest">Backend Status</span>
                        <div className="flex items-center gap-2">
                            <span className={`w-2 h-2 rounded-full ${systemStatus?.status === 'ready' ? 'bg-green-500 shadow-[0_0_8px_rgba(34,197,94,0.4)]' : systemStatus?.status === 'error' ? 'bg-red-500' : 'bg-amber-500 animate-pulse'}`}></span>
                            <span className={`text-sm font-semibold ${systemStatus?.status === 'ready' ? 'text-green-500' : systemStatus?.status === 'error' ? 'text-red-500' : 'text-amber-500'}`}>
                                {systemStatus?.status?.toUpperCase() || 'CONNECTING...'}
                            </span>
                        </div>
                    </div>
                    <div className="flex flex-col gap-1.5 p-4 bg-[var(--bg-app)] rounded-lg border border-[var(--border-color)]/50">
                        <span className="text-[10px] font-bold text-[var(--text-muted)] uppercase tracking-widest">Embedding Model</span>
                        <span className="text-sm font-medium text-[var(--text-primary)]">
                            {systemStatus?.status === 'initializing'
                                ? (systemStatus.progress || 'Initializing...')
                                : (systemStatus?.status === 'ready' ? 'all-MiniLM-L6-v2' : 'Unknown')}
                        </span>
                    </div>
                    <div className="flex flex-col gap-1.5 p-4 bg-[var(--bg-app)] rounded-lg border border-[var(--border-color)]/50">
                        <span className="text-[10px] font-bold text-[var(--text-muted)] uppercase tracking-widest">Active Jobs</span>
                        <span className="text-sm font-medium text-[var(--text-primary)]">
                            {jobs.filter(j => j.status === 'running' || j.status === 'pending').length}
                        </span>
                    </div>
                </div>
            </section>

            <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-xl overflow-hidden shadow-sm">
                <div className="flex items-center gap-2.5 px-5 py-3.5 bg-[var(--item-hover)] border-b border-[var(--border-color)]">
                    <span className="text-base">📜</span>
                    <h3 className="m-0 text-sm font-semibold text-[var(--text-active)] border-none p-0">Job History</h3>
                </div>
                <div className="p-0">
                    {jobs.length === 0 ? (
                        <div className="p-12 text-center flex flex-col items-center gap-3">
                            <span className="text-4xl opacity-20">📥</span>
                            <p className="text-[var(--text-secondary)] text-sm italic">No activity yet. Add a source and click "Index now" to get started!</p>
                        </div>
                    ) : (
                        <div className="overflow-x-auto">
                            <table className="w-full text-left border-collapse">
                                <thead>
                                    <tr className="bg-[var(--bg-app)]/50 border-b border-[var(--border-color)]">
                                        <th className="px-6 py-3 text-[10px] font-bold text-[var(--text-muted)] uppercase tracking-widest">Source</th>
                                        <th className="px-6 py-3 text-[10px] font-bold text-[var(--text-muted)] uppercase tracking-widest">Status</th>
                                        <th className="px-6 py-3 text-[10px] font-bold text-[var(--text-muted)] uppercase tracking-widest text-center">Files</th>
                                        <th className="px-6 py-3 text-[10px] font-bold text-[var(--text-muted)] uppercase tracking-widest text-center">Chunks</th>
                                        <th className="px-6 py-3 text-[10px] font-bold text-[var(--text-muted)] uppercase tracking-widest">Time</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {jobs.map((job) => (
                                        <tr key={job.id} className="border-b border-[var(--border-color)]/50 hover:bg-[var(--item-hover)] transition-colors group">
                                            <td className="px-6 py-4">
                                                <div className="flex flex-col gap-1">
                                                    <span className="text-sm font-medium text-[var(--text-primary)] group-hover:text-[var(--text-active)]">{job.sourceName}</span>
                                                    {job.sourceType && <span className="text-[9px] w-fit bg-[var(--bg-button)] px-1.5 py-0.5 rounded text-[var(--text-muted)] border border-[var(--border-color)] uppercase font-bold tracking-tighter">{job.sourceType}</span>}
                                                </div>
                                            </td>
                                            <td className="px-6 py-4">
                                                <div className="flex flex-col gap-1">
                                                    <span className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-[10px] font-bold w-fit ${
                                                        job.status === 'pending' ? 'bg-amber-500/10 text-amber-500 border border-amber-500/20' :
                                                        job.status === 'running' ? 'bg-blue-500/10 text-blue-500 border border-blue-500/20 animate-pulse' :
                                                        job.status === 'completed' ? 'bg-green-500/10 text-green-500 border border-green-500/20' :
                                                        'bg-red-500/10 text-red-500 border border-red-500/20'
                                                    }`}>
                                                        {job.status === 'pending' && '⏳ PENDING'}
                                                        {job.status === 'running' && '● RUNNING'}
                                                        {job.status === 'completed' && '✓ COMPLETED'}
                                                        {job.status === 'error' && '⚠ ERROR'}
                                                    </span>
                                                    {job.error && <span className="text-[10px] text-red-400/80 max-w-[200px] truncate" title={job.error}>{job.error}</span>}
                                                </div>
                                            </td>
                                            <td className="px-6 py-4 text-center text-sm font-mono text-[var(--text-secondary)]">{job.filesIndexed ?? '—'}</td>
                                            <td className="px-6 py-4 text-center text-sm font-mono text-[var(--text-secondary)]">{job.chunksCreated ?? '—'}</td>
                                            <td className="px-6 py-4">
                                                <div className="flex flex-col text-[10px] text-[var(--text-muted)] font-medium">
                                                    <span>Start: {new Date(job.startedAt).toLocaleTimeString()}</span>
                                                    {job.finishedAt && <span>End: {new Date(job.finishedAt).toLocaleTimeString()}</span>}
                                                </div>
                                            </td>
                                        </tr>
                                    ))}
                                </tbody>
                            </table>
                        </div>
                    )}
                </div>
            </section>
        </div>
    )
}
