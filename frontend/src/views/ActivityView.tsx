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
        const interval = setInterval(fetchStatus, 3000)
        return () => clearInterval(interval)
    }, [])

    return (
        <div className="flex-1 overflow-y-auto w-full bg-[var(--bg-app)]">
            <div className="max-w-[1000px] mx-auto p-8 flex flex-col gap-10 pb-32">
                
                <header className="flex flex-col gap-2">
                    <h2 className="text-3xl font-extrabold text-[var(--text-active)] tracking-tight">Activity</h2>
                    <p className="text-sm text-[var(--text-secondary)] opacity-80 font-medium">Monitor system health and indexing history.</p>
                </header>

                <div className="flex flex-col gap-8">
                    {/* System Status Cards */}
                    <section className="grid grid-cols-1 md:grid-cols-3 gap-4">
                        <div className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl p-6 shadow-sm flex flex-col gap-4 transition-all hover:shadow-md">
                            <div className="flex items-center justify-between">
                                <span className="text-[10px] font-black text-[var(--text-muted)] uppercase tracking-widest">Backend</span>
                                <span className="text-lg">🖥️</span>
                            </div>
                            <div className="flex flex-col gap-1">
                                <div className="flex items-center gap-2">
                                    <span className={`w-2.5 h-2.5 rounded-full ${systemStatus?.status === 'ready' ? 'bg-green-500 shadow-[0_0_10px_rgba(34,197,94,0.4)]' : systemStatus?.status === 'error' ? 'bg-red-500' : 'bg-amber-500 animate-pulse'}`}></span>
                                    <span className={`text-sm font-black ${systemStatus?.status === 'ready' ? 'text-green-500' : systemStatus?.status === 'error' ? 'text-red-500' : 'text-amber-500'}`}>
                                        {systemStatus?.status?.toUpperCase() || 'CONNECTING...'}
                                    </span>
                                </div>
                                <span className="text-[10px] text-[var(--text-muted)] font-bold">CORE ENGINE STATUS</span>
                            </div>
                        </div>

                        <div className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl p-6 shadow-sm flex flex-col gap-4 transition-all hover:shadow-md">
                            <div className="flex items-center justify-between">
                                <span className="text-[10px] font-black text-[var(--text-muted)] uppercase tracking-widest">Model</span>
                                <span className="text-lg">🧠</span>
                            </div>
                            <div className="flex flex-col gap-1">
                                <span className="text-sm font-black text-[var(--text-primary)] truncate">
                                    {systemStatus?.status === 'initializing'
                                        ? (systemStatus.progress || 'INITIALIZING...')
                                        : (systemStatus?.status === 'ready' ? 'all-MiniLM-L6-v2' : 'OFFLINE')}
                                </span>
                                <span className="text-[10px] text-[var(--text-muted)] font-bold">EMBEDDING ENGINE</span>
                            </div>
                        </div>

                        <div className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl p-6 shadow-sm flex flex-col gap-4 transition-all hover:shadow-md">
                            <div className="flex items-center justify-between">
                                <span className="text-[10px] font-black text-[var(--text-muted)] uppercase tracking-widest">Jobs</span>
                                <span className="text-lg">⚙️</span>
                            </div>
                            <div className="flex flex-col gap-1">
                                <span className="text-sm font-black text-[var(--text-primary)]">
                                    {jobs.filter(j => j.status === 'running' || j.status === 'pending').length} ACTIVE
                                </span>
                                <span className="text-[10px] text-[var(--text-muted)] font-bold">CURRENT TASK QUEUE</span>
                            </div>
                        </div>
                    </section>

                    {/* Job History */}
                    <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl shadow-sm overflow-hidden flex flex-col">
                        <div className="flex items-center justify-between px-6 py-4 bg-[var(--item-hover)]/30 border-b border-[var(--border-color)]">
                            <div className="flex items-center gap-3">
                                <span className="text-lg">📜</span>
                                <h3 className="text-sm font-bold text-[var(--text-active)]">Job History</h3>
                            </div>
                            <span className="text-[10px] font-black text-[var(--text-muted)] bg-[var(--bg-app)] px-2 py-1 rounded-lg border border-[var(--border-color)]">
                                SHOWING LAST {jobs.length} ACTIVITIES
                            </span>
                        </div>
                        
                        <div className="overflow-x-auto">
                            {jobs.length === 0 ? (
                                <div className="p-20 text-center flex flex-col items-center gap-4">
                                    <div className="w-16 h-16 bg-[var(--bg-app)] rounded-full flex items-center justify-center text-2xl border-2 border-dashed border-[var(--border-color)] opacity-50">📥</div>
                                    <div className="flex flex-col gap-1">
                                        <p className="text-sm font-bold text-[var(--text-primary)]">No activity logs found</p>
                                        <p className="text-xs text-[var(--text-muted)]">Index a source to see its progress here.</p>
                                    </div>
                                </div>
                            ) : (
                                <table className="w-full text-left border-collapse">
                                    <thead>
                                        <tr className="bg-[var(--bg-app)]/50 border-b border-[var(--border-color)]">
                                            <th className="px-6 py-4 text-[10px] font-black text-[var(--text-muted)] uppercase tracking-widest">Source & Type</th>
                                            <th className="px-6 py-4 text-[10px] font-black text-[var(--text-muted)] uppercase tracking-widest">Status</th>
                                            <th className="px-6 py-4 text-[10px] font-black text-[var(--text-muted)] uppercase tracking-widest text-center">Files / Chunks</th>
                                            <th className="px-6 py-4 text-[10px] font-black text-[var(--text-muted)] uppercase tracking-widest text-right">Timestamp</th>
                                        </tr>
                                    </thead>
                                    <tbody className="divide-y divide-[var(--border-color)]/30">
                                        {jobs.map((job) => (
                                            <tr key={job.id} className="hover:bg-[var(--item-hover)] transition-colors group">
                                                <td className="px-6 py-5">
                                                    <div className="flex flex-col gap-1.5">
                                                        <span className="text-sm font-bold text-[var(--text-primary)] group-hover:text-[var(--text-active)] tracking-tight">{job.sourceName}</span>
                                                        <span className="text-[9px] w-fit bg-[var(--bg-app)] px-2 py-0.5 rounded border border-[var(--border-color)] text-[var(--text-muted)] font-black tracking-tighter uppercase">{job.sourceType || 'LOCAL'}</span>
                                                    </div>
                                                </td>
                                                <td className="px-6 py-5">
                                                    <div className="flex flex-col gap-2">
                                                        <div className={`inline-flex items-center gap-2 px-3 py-1 rounded-full text-[10px] font-black w-fit border-2 ${
                                                            job.status === 'pending' ? 'bg-amber-500/5 text-amber-500 border-amber-500/20' :
                                                            job.status === 'running' ? 'bg-blue-500/5 text-blue-500 border-blue-500/20 animate-pulse' :
                                                            job.status === 'completed' ? 'bg-green-500/5 text-green-500 border-green-500/20' :
                                                            'bg-red-500/5 text-red-500 border-red-500/20'
                                                        }`}>
                                                            <span className={`w-1.5 h-1.5 rounded-full ${
                                                                job.status === 'pending' ? 'bg-amber-500' :
                                                                job.status === 'running' ? 'bg-blue-500' :
                                                                job.status === 'completed' ? 'bg-green-500' :
                                                                'bg-red-500'
                                                            }`}></span>
                                                            {job.status.toUpperCase()}
                                                        </div>
                                                        {job.error && (
                                                            <div className="bg-red-500/5 border border-red-500/10 p-2 rounded-lg max-w-[250px]">
                                                                <p className="text-[10px] text-red-400 font-medium leading-relaxed italic break-words">{job.error}</p>
                                                            </div>
                                                        )}
                                                    </div>
                                                </td>
                                                <td className="px-6 py-5">
                                                    <div className="flex flex-col items-center gap-1">
                                                        <div className="flex items-center gap-2">
                                                            <span className="text-xs font-black text-[var(--text-primary)] font-mono">{job.filesIndexed ?? 0}</span>
                                                            <span className="text-[10px] text-[var(--text-muted)] font-bold">FILES</span>
                                                        </div>
                                                        <div className="h-px w-8 bg-[var(--border-color)]/30"></div>
                                                        <div className="flex items-center gap-2">
                                                            <span className="text-xs font-black text-[var(--text-muted)] font-mono">{job.chunksCreated ?? 0}</span>
                                                            <span className="text-[10px] text-[var(--text-muted)] font-bold">CHUNKS</span>
                                                        </div>
                                                    </div>
                                                </td>
                                                <td className="px-6 py-5 text-right">
                                                    <div className="flex flex-col gap-1 font-mono">
                                                        <div className="flex items-center justify-end gap-2">
                                                            <span className="text-[10px] text-[var(--text-muted)] font-bold uppercase">Start</span>
                                                            <span className="text-[11px] text-[var(--text-primary)]">{new Date(job.startedAt).toLocaleTimeString()}</span>
                                                        </div>
                                                        {job.finishedAt && (
                                                            <div className="flex items-center justify-end gap-2">
                                                                <span className="text-[10px] text-[var(--text-muted)] font-bold uppercase tracking-tighter">Finish</span>
                                                                <span className="text-[11px] text-[var(--text-secondary)]">{new Date(job.finishedAt).toLocaleTimeString()}</span>
                                                            </div>
                                                        )}
                                                    </div>
                                                </td>
                                            </tr>
                                        ))}
                                    </tbody>
                                </table>
                            )}
                        </div>
                    </section>
                </div>
            </div>
        </div>
    )
}
