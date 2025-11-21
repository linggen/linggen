import { useState, useEffect } from 'react'
import './App.css'
import {
  indexDocument,
  indexSource,
  searchDocuments,
  listJobs,
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

type JobStatus = 'running' | 'completed' | 'error'

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
          status: job.status === 'Running' ? 'running' : job.status === 'Completed' ? 'completed' : 'error',
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
            setIndexingProgress('Indexing in progress...')
          }
        }
      } catch (error) {
        console.error('Failed to fetch jobs:', error)
      }
    }

    fetchJobs()
  }, [])

  const handleIndexResource = async (resource: Resource) => {
    if (resource.resource_type !== 'local') {
      // For now we only support indexing local folders
      return
    }

    setIndexingResourceId(resource.id)
    setIndexingProgress('Starting...')
    setStatus('indexing')

    try {
      // Start indexing via new API
      const result = await indexSource(resource.id)
      const jobId = result.job_id

      // Poll for job status
      const pollInterval = setInterval(async () => {
        try {
          const response = await listJobs()
          const job = response.jobs.find((j) => j.id === jobId)
          
          if (job) {
            // Update progress based on job status
            if (job.status === 'Running') {
              const filesIndexed = job.files_indexed || 0
              const chunksCreated = job.chunks_created || 0
              if (filesIndexed > 0) {
                setIndexingProgress(`Processing... ${filesIndexed} files, ${chunksCreated} chunks`)
              } else {
                setIndexingProgress('Reading files...')
              }
            } else if (job.status === 'Completed') {
              clearInterval(pollInterval)
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
                setStatus('idle')
              }, 3000)
              
              setCurrentView('activity')
            } else if (job.status === 'Failed') {
              clearInterval(pollInterval)
              setIndexingProgress(`✗ Error: ${job.error || 'Unknown error'}`)
              
              // Update jobs list
              const frontendJob: Job = {
                id: job.id,
                sourceId: job.source_id,
                sourceName: job.source_name,
                sourceType: job.source_type,
                startedAt: job.started_at,
                finishedAt: job.finished_at,
                status: 'error',
                error: job.error,
              }
              setJobs((prev) => [frontendJob, ...prev.filter((j) => j.id !== jobId)])
              
              setTimeout(() => {
                setIndexingResourceId(null)
                setIndexingProgress(null)
                setStatus('error')
              }, 3000)
              
              setCurrentView('activity')
            }
          }
        } catch (error) {
          console.error('Failed to poll job status:', error)
        }
      }, 1000) // Poll every second

      // Stop polling after 10 minutes (safety timeout)
      setTimeout(() => {
        clearInterval(pollInterval)
        if (indexingResourceId === resource.id) {
          setIndexingProgress('✗ Timeout')
          setIndexingResourceId(null)
          setStatus('error')
        }
      }, 600000)

    } catch (error) {
      setIndexingProgress(`✗ Error: ${error}`)
      setTimeout(() => {
        setIndexingResourceId(null)
        setIndexingProgress(null)
        setStatus('error')
      }, 3000)
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
}

function SourcesView({ onIndexResource, indexingResourceId, indexingProgress }: SourcesViewProps) {
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
