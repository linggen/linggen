import { useState, useEffect, useRef, useCallback } from 'react'
import './App.css'
import {
  indexDocument,
  indexSource,
  searchDocuments,
  listJobs,
  cancelJob,
  type Resource,
  type ResourceType,
  type SearchResult,
} from './api'
import { ResourceManager } from './ResourceManager'

// Core flows:
// 1) Manage sources (git/local/web) via Sources view
// 2) Index content from sources (currently local folders) into LanceDB
// 3) Search across indexed content via a focused Search view

type View = 'sources' | 'search' | 'activity' | 'settings'

type JobStatus = 'pending' | 'running' | 'completed' | 'error'

interface Job {
  id: string
  sourceId?: string
  sourceName: string
  sourceType?: ResourceType
  startedAt: string
  finishedAt?: string
  status: JobStatus
  filesIndexed?: number
  chunksCreated?: number
  error?: string
}

type AppStatus = 'idle' | 'indexing' | 'error'

function App() {
  const [currentView, setCurrentView] = useState<View>('search')
  const [status, setStatus] = useState<AppStatus>('idle')
  const [jobs, setJobs] = useState<Job[]>([])
  const [indexingResourceId, setIndexingResourceId] = useState<string | null>(null)
  const [indexingProgress, setIndexingProgress] = useState<string | null>(null)
  const [currentJobId, setCurrentJobId] = useState<string | null>(null)

  // Use ref to track polling interval and cancelling state
  const pollingIntervalRef = useRef<number | null>(null)
  const pollingTimeoutRef = useRef<number | null>(null)
  const isCancellingRef = useRef<boolean>(false)

  // Shared function to update progress for a running job
  const updateJobProgress = useCallback(async (jobId: string) => {
    try {
      const response = await listJobs()
      const job = response.jobs.find((j) => j.id === jobId)

      if (job) {
        // Update progress based on job status
        if (job.status === 'Pending') {
          setIndexingProgress('⏳ Waiting in queue...')
        } else if (job.status === 'Running') {
          // Don't update progress if we're in the middle of cancelling
          if (isCancellingRef.current) {
            return
          }

          const filesIndexed = job.files_indexed || 0
          const totalFiles = job.total_files || 0
          const totalSizeBytes = job.total_size_bytes || 0
          const chunksCreated = job.chunks_created || 0

          // Format size in MB or GB
          const formatSize = (bytes: number) => {
            if (bytes >= 1_000_000_000) {
              return `${(bytes / 1_000_000_000).toFixed(2)} GB`
            } else if (bytes >= 1_000_000) {
              return `${(bytes / 1_000_000).toFixed(2)} MB`
            } else if (bytes >= 1_000) {
              return `${(bytes / 1_000).toFixed(2)} KB`
            } else {
              return `${bytes} bytes`
            }
          }

          if (totalFiles > 0 && filesIndexed > 0) {
            const percentage = Math.round((filesIndexed / totalFiles) * 100)
            const sizeStr = totalSizeBytes > 0 ? ` (${formatSize(totalSizeBytes)})` : ''
            setIndexingProgress(`${percentage}% - ${filesIndexed}/${totalFiles} files${sizeStr}`)
          } else if (filesIndexed > 0) {
            setIndexingProgress(`Processing... ${filesIndexed} files, ${chunksCreated} chunks`)
          } else {
            setIndexingProgress('Reading files...')
          }
        } else if (job.status === 'Completed') {
          // Stop polling
          if (pollingIntervalRef.current) {
            clearInterval(pollingIntervalRef.current)
            pollingIntervalRef.current = null
          }
          if (pollingTimeoutRef.current) {
            clearTimeout(pollingTimeoutRef.current)
            pollingTimeoutRef.current = null
          }

          setIndexingProgress(`✓ Indexed ${job.files_indexed} files, ${job.chunks_created} chunks`)

          // Update jobs list
          const frontendJob: Job = {
            id: job.id,
            sourceId: job.source_id,
            sourceName: job.source_name,
            sourceType: job.source_type,
            startedAt: job.started_at,
            finishedAt: job.finished_at,
            status: 'completed',
            filesIndexed: job.files_indexed,
            chunksCreated: job.chunks_created,
          }
          setJobs((prev) => [frontendJob, ...prev.filter((j) => j.id !== jobId)])

          setTimeout(() => {
            setIndexingResourceId(null)
            setIndexingProgress(null)
            setCurrentJobId(null)
            setStatus('idle')
          }, 3000)

          setCurrentView('activity')
        } else if (job.status === 'Failed') {
          // Stop polling
          if (pollingIntervalRef.current) {
            clearInterval(pollingIntervalRef.current)
            pollingIntervalRef.current = null
          }
          if (pollingTimeoutRef.current) {
            clearTimeout(pollingTimeoutRef.current)
            pollingTimeoutRef.current = null
          }

          const errorMsg = job.error || 'Unknown error'
          const isCancelled = errorMsg.includes('cancelled')
          setIndexingProgress(isCancelled ? '✓ Cancelled' : `✗ ${errorMsg}`)

          // Update jobs list
          const frontendJob: Job = {
            id: job.id,
            sourceId: job.source_id,
            sourceName: job.source_name,
            sourceType: job.source_type,
            startedAt: job.started_at,
            finishedAt: job.finished_at,
            status: errorMsg.includes('cancelled') ? 'completed' : 'error',
            error: job.error,
          }
          setJobs((prev) => [frontendJob, ...prev.filter((j) => j.id !== jobId)])

          setTimeout(() => {
            setIndexingResourceId(null)
            setIndexingProgress(null)
            setCurrentJobId(null)
            isCancellingRef.current = false
            setStatus(errorMsg.includes('cancelled') ? 'idle' : 'error')
          }, 3000)

          setCurrentView('activity')
        }
      }
    } catch (error) {
      console.error('Failed to poll job status:', error)
    }
  }, [])

  // Start polling for a job
  const startPollingJob = useCallback((jobId: string) => {
    // Clear any existing polling
    if (pollingIntervalRef.current) {
      clearInterval(pollingIntervalRef.current)
    }
    if (pollingTimeoutRef.current) {
      clearTimeout(pollingTimeoutRef.current)
    }

    // Start polling every second
    pollingIntervalRef.current = setInterval(() => {
      updateJobProgress(jobId)
    }, 1000)

    // Safety timeout after 10 minutes
    pollingTimeoutRef.current = setTimeout(() => {
      if (pollingIntervalRef.current) {
        clearInterval(pollingIntervalRef.current)
        pollingIntervalRef.current = null
      }
      setIndexingProgress('✗ Timeout')
      setIndexingResourceId(null)
      setCurrentJobId(null)
      setStatus('error')
    }, 600000)
  }, [updateJobProgress])

  // Fetch jobs from backend on startup
  useEffect(() => {
    const fetchJobs = async () => {
      try {
        const response = await listJobs()
        const backendJobs: Job[] = response.jobs.map((job) => ({
          id: job.id,
          sourceId: job.source_id,
          sourceName: job.source_name,
          sourceType: job.source_type,
          startedAt: job.started_at,
          finishedAt: job.finished_at,
          status: job.status === 'Running' ? 'running' : job.status === 'Pending' ? 'pending' : job.status === 'Completed' ? 'completed' : 'error',
          filesIndexed: job.files_indexed,
          chunksCreated: job.chunks_created,
          error: job.error,
        }))
        setJobs(backendJobs)

        // Check if any job is still running
        const hasRunningJob = backendJobs.some((job) => job.status === 'running')
        if (hasRunningJob) {
          setStatus('indexing')
          // Find the running job and set indexing state
          const runningJob = backendJobs.find((job) => job.status === 'running')
          if (runningJob?.sourceId) {
            setIndexingResourceId(runningJob.sourceId)
            setCurrentJobId(runningJob.id) // Set the job ID for cancellation
            setIndexingProgress('Indexing in progress...')
          }
        }
      } catch (error) {
        console.error('Failed to fetch jobs:', error)
      }
    }

    fetchJobs()
  }, [])

  // Start polling when we detect a running job (e.g., on page load)
  useEffect(() => {
    if (currentJobId && status === 'indexing') {
      startPollingJob(currentJobId)
    }

    // Cleanup on unmount
    return () => {
      if (pollingIntervalRef.current) {
        clearInterval(pollingIntervalRef.current)
      }
      if (pollingTimeoutRef.current) {
        clearTimeout(pollingTimeoutRef.current)
      }
    }
  }, [currentJobId, status, startPollingJob])

  const handleIndexResource = async (resource: Resource) => {
    if (resource.resource_type !== 'local') {
      // For now we only support indexing local folders
      return
    }

    setIndexingResourceId(resource.id)
    setIndexingProgress('Indexing...')
    setStatus('indexing')

    try {
      // Start indexing via new API
      const result = await indexSource(resource.id)
      const jobId = result.job_id
      setCurrentJobId(jobId) // Track current job (this will trigger the polling useEffect)

    } catch (error) {
      setIndexingProgress(`✗ Error: ${error}`)
      setTimeout(() => {
        setIndexingResourceId(null)
        setIndexingProgress(null)
        setCurrentJobId(null)
        setStatus('error')
      }, 3000)
    }
  }

  const handleCancelJob = async () => {
    console.log('Cancel button clicked!')
    console.log('  currentJobId:', currentJobId)
    console.log('  indexingResourceId:', indexingResourceId)
    console.log('  jobs:', jobs)

    let jobIdToCancel = currentJobId

    // If no currentJobId but we have an indexing resource, find the running job
    if (!jobIdToCancel && indexingResourceId) {
      console.log('No currentJobId, looking for running job with resourceId:', indexingResourceId)
      console.log('  Available jobs:', jobs.map(j => ({ id: j.id, sourceId: j.sourceId, status: j.status })))

      const runningJob = jobs.find(
        (job) => job.sourceId === indexingResourceId && job.status === 'running'
      )

      if (runningJob) {
        jobIdToCancel = runningJob.id
        console.log('Found running job:', jobIdToCancel)
      } else {
        console.warn('No running job found for resourceId:', indexingResourceId)
        console.log('  Jobs with matching resourceId:', jobs.filter(j => j.sourceId === indexingResourceId))
        console.log('  Running jobs:', jobs.filter(j => j.status === 'running'))
      }
    }

    if (!jobIdToCancel) {
      console.warn('No job ID to cancel - cannot cancel')
      setIndexingProgress('✗ No active job found')
      return
    }

    // Set cancelling flag immediately to prevent progress updates
    isCancellingRef.current = true
    setIndexingProgress('Cancelling...')

    try {
      console.log('Calling cancelJob API for job:', jobIdToCancel)
      await cancelJob(jobIdToCancel)
      console.log('Cancel request sent successfully')

      // Check job status to see if it was actually cancelled
      setTimeout(async () => {
        try {
          const response = await listJobs()
          const job = response.jobs.find((j) => j.id === jobIdToCancel)

          if (job && job.status === 'Failed' && job.error?.includes('cancelled')) {
            console.log('Job was successfully cancelled')
            setIndexingProgress('✓ Cancelled')
            setTimeout(() => {
              setIndexingResourceId(null)
              setIndexingProgress(null)
              setCurrentJobId(null)
              setStatus('idle')
            }, 2000)
          } else if (job && job.status !== 'Running') {
            console.log('Job already finished with status:', job.status)
            setIndexingProgress(`Job already ${job.status}`)
            setTimeout(() => {
              setIndexingResourceId(null)
              setIndexingProgress(null)
              setCurrentJobId(null)
              setStatus('idle')
            }, 2000)
          } else {
            console.log('Job still running, will be cancelled soon')
          }
        } catch (error) {
          console.error('Failed to check job status after cancel:', error)
        }
      }, 1000)
    } catch (error) {
      console.error('Failed to cancel job:', error)
      setIndexingProgress(`✗ Failed to cancel: ${error}`)
    }
  }

  return (
    <div className="app">
      <header className="app-header">
        <div>
          <h1>🧠 RememberMe</h1>
          <p>Your personal knowledge hub. Search everything, instantly.</p>
        </div>
        <StatusBadge status={status} />
      </header>

      <div className="layout">
        <Sidebar currentView={currentView} onChangeView={setCurrentView} />

        <main className="main">
          {currentView === 'sources' && (
            <SourcesView
              onIndexResource={handleIndexResource}
              indexingResourceId={indexingResourceId}
              indexingProgress={indexingProgress}
              onCancelJob={handleCancelJob}
            />
          )}
          {currentView === 'search' && <SearchView />}
          {currentView === 'activity' && <ActivityView jobs={jobs} />}
          {currentView === 'settings' && <SettingsView />}
        </main>
      </div>
    </div>
  )
}

interface SidebarProps {
  currentView: View
  onChangeView: (view: View) => void
}

function Sidebar({ currentView, onChangeView }: SidebarProps) {
  const items: { id: View; label: string; description: string }[] = [
    { id: 'sources', label: 'Sources', description: 'Your knowledge sources' },
    { id: 'search', label: 'Search', description: 'Find anything, instantly' },
    { id: 'activity', label: 'Activity', description: 'Recent indexing jobs' },
    { id: 'settings', label: 'Settings', description: 'App configuration' },
  ]

  return (
    <nav className="sidebar">
      <div className="sidebar-section">
        <span className="sidebar-section-title">Navigation</span>
        <ul className="sidebar-list">
          {items.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                className={`sidebar-item ${currentView === item.id ? 'active' : ''}`}
                onClick={() => onChangeView(item.id)}
              >
                <div className="sidebar-item-main">
                  <span className="sidebar-item-label">{item.label}</span>
                  <span className="sidebar-item-description">{item.description}</span>
                </div>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </nav>
  )
}

interface StatusBadgeProps {
  status: AppStatus
}

function StatusBadge({ status }: StatusBadgeProps) {
  let text = 'Idle'
  let className = 'status-pill idle'

  if (status === 'indexing') {
    text = 'Indexing'
    className = 'status-pill indexing'
  } else if (status === 'error') {
    text = 'Error'
    className = 'status-pill error'
  }

  return (
    <div className={className}>
      <span className="status-dot" />
      <span>{text}</span>
    </div>
  )
}

interface SourcesViewProps {
  onIndexResource: (resource: Resource) => void
  indexingResourceId: string | null
  indexingProgress: string | null
  onCancelJob: () => void
}

function SourcesView({ onIndexResource, indexingResourceId, indexingProgress, onCancelJob }: SourcesViewProps) {
  return (
    <div className="view">
      <div className="view-header">
        <h2>Sources</h2>
        <p>Add folders, repositories, and websites to search across all your knowledge.</p>
      </div>
      <section className="section">
        <ResourceManager
          onIndexResource={onIndexResource}
          indexingResourceId={indexingResourceId}
          indexingProgress={indexingProgress}
          onCancelJob={onCancelJob}
        />
      </section>
    </div>
  )
}

function SearchView() {
  const [docId, setDocId] = useState('')
  const [content, setContent] = useState('')
  const [indexing, setIndexing] = useState(false)
  const [indexStatus, setIndexStatus] = useState('')

  const [query, setQuery] = useState('')
  const [searching, setSearching] = useState(false)
  const [results, setResults] = useState<SearchResult[]>([])
  const [searchError, setSearchError] = useState('')

  const handleIndex = async (e: React.FormEvent) => {
    e.preventDefault()
    setIndexing(true)
    setIndexStatus('')

    try {
      const response = await indexDocument({
        document_id: docId,
        content,
      })
      setIndexStatus(`✓ Indexed ${response.chunks_indexed} chunks for document: ${response.document_id}`)
      setDocId('')
      setContent('')
    } catch (error) {
      setIndexStatus(`✗ Error: ${error}`)
    } finally {
      setIndexing(false)
    }
  }

  const handleSearch = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!query.trim()) return

    setSearching(true)
    setSearchError('')

    try {
      const response = await searchDocuments(query, 10)
      setResults(response.results)
      if (response.results.length === 0) {
        setSearchError('No results found. Try indexing some documents first!')
      }
    } catch (error) {
      setSearchError(`Search failed: ${error}`)
      setResults([])
    } finally {
      setSearching(false)
    }
  }

  return (
    <div className="view">
      <div className="view-header">
        <h2>Search</h2>
        <p>Find anything across all your sources. Ask questions in natural language.</p>
      </div>

      <div className="search-layout">
        <section className="section">
          <h3>📥 Index a single document</h3>
          <form onSubmit={handleIndex}>
            <div className="form-group">
              <label htmlFor="docId">Document ID</label>
              <input
                id="docId"
                type="text"
                value={docId}
                onChange={(e) => setDocId(e.target.value)}
                placeholder="e.g., doc1, my-notes, etc."
                required
              />
            </div>
            <div className="form-group">
              <label htmlFor="content">Content</label>
              <textarea
                id="content"
                value={content}
                onChange={(e) => setContent(e.target.value)}
                placeholder="Paste your document content here..."
                rows={6}
                required
              />
            </div>
            <button type="submit" disabled={indexing}>
              {indexing ? 'Indexing...' : 'Index Document'}
            </button>
          </form>
          {indexStatus && (
            <div className={`status ${indexStatus.startsWith('✓') ? 'success' : 'error'}`}>
              {indexStatus}
            </div>
          )}
        </section>

        <section className="section">
          <h3>🔍 Search indexed content</h3>
          <form onSubmit={handleSearch}>
            <div className="form-group">
              <label htmlFor="query">Search query</label>
              <input
                id="query"
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Ask about your notes, code, or docs..."
                required
              />
            </div>
            <button type="submit" disabled={searching}>
              {searching ? 'Searching...' : 'Search'}
            </button>
          </form>

          {searchError && <div className="status error">{searchError}</div>}

          {results.length > 0 && (
            <div className="results">
              <h3>Results ({results.length})</h3>
              {results.map((result, idx) => (
                <div key={idx} className="result-card">
                  <div className="result-header">
                    <span className="doc-id">{result.document_id}</span>
                    <span className="score">Score: {result.score.toFixed(3)}</span>
                  </div>
                  <p className="result-content">{result.content}</p>
                </div>
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  )
}

interface ActivityViewProps {
  jobs: Job[]
}

function ActivityView({ jobs }: ActivityViewProps) {
  return (
    <div className="view">
      <div className="view-header">
        <h2>Activity</h2>
        <p>Track your indexing progress and view completed jobs.</p>
      </div>

      <section className="section">
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

function SettingsView() {
  return (
    <div className="view">
      <div className="view-header">
        <h2>Settings</h2>
        <p>Configure how RememberMe stores and processes your data.</p>
      </div>

      <section className="section settings-section">
        <div className="settings-group">
          <h3>Data Storage</h3>
          <div className="settings-item">
            <span className="settings-label">Search index</span>
            <span className="settings-value">./backend/data/lancedb</span>
          </div>
          <div className="settings-item">
            <span className="settings-label">Source metadata</span>
            <span className="settings-value">./backend/data/metadata.redb</span>
          </div>
        </div>

        <div className="settings-group">
          <h3>Search Engine</h3>
          <div className="settings-item">
            <span className="settings-label">AI Model</span>
            <span className="settings-value">all-MiniLM-L6-v2</span>
          </div>
          <div className="settings-item">
            <span className="settings-label">Privacy</span>
            <span className="settings-value">100% local, offline-capable, your data never leaves your device</span>
          </div>
        </div>
      </section>
    </div>
  )
}

export default App
