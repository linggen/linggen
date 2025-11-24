import { useState, useEffect, useRef, useCallback } from 'react'
import './App.css'
import {
  indexSource,
  listJobs,
  cancelJob,
  enhancePrompt,
  getPreferences,
  updatePreferences,
  getAppStatus,
  retryInit,
  type Resource,
  type ResourceType,
  type IntentType,
  type UserPreferences,
  type PromptStrategy,
} from './api'
import { ResourceManager } from './ResourceManager'
import { SourceProfile } from './components/ProjectProfile'

// Core flows:
// 1) Manage sources (git/local/web) via Sources view
// 2) Index content from sources (currently local folders) into LanceDB
// 3) AI Assistant for intent classification and prompt enhancement

type View = 'sources' | 'activity' | 'assistant' | 'settings'

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

type AppStatus = 'initializing' | 'idle' | 'indexing' | 'error'

function App() {
  const [currentView, setCurrentView] = useState<View>('assistant')
  const [status, setStatus] = useState<AppStatus>('initializing')
  const [statusMessage, setStatusMessage] = useState<string | null>(null)
  const [jobs, setJobs] = useState<Job[]>([])
  const [indexingResourceId, setIndexingResourceId] = useState<string | null>(null)
  const [indexingProgress, setIndexingProgress] = useState<string | null>(null)
  const [currentJobId, setCurrentJobId] = useState<string | null>(null)
  const [selectedSourceId, setSelectedSourceId] = useState<string | null>(null)

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

  // Check app initialization status on startup
  useEffect(() => {
    const checkAppStatus = async () => {
      try {
        const response = await getAppStatus()
        if (response.status === 'initializing') {
          setStatus('initializing')
          setStatusMessage(response.progress || response.message || 'Initializing...')
        } else if (response.status === 'ready') {
          setStatus('idle')
          setStatusMessage(null)
        } else if (response.status === 'error') {
          setStatus('error')
          setStatusMessage(response.message || 'Initialization failed')
        }
      } catch (error) {
        console.error('Failed to check app status:', error)
      }
    }

    checkAppStatus()

    // Poll status every 2 seconds until ready
    const statusInterval = setInterval(async () => {
      try {
        const response = await getAppStatus()
        if (response.status === 'ready') {
          setStatus('idle')
          setStatusMessage(null)
          clearInterval(statusInterval)
        } else if (response.status === 'initializing') {
          setStatus('initializing')
          setStatusMessage(response.progress || response.message || 'Initializing...')
        } else if (response.status === 'error') {
          setStatus('error')
          setStatusMessage(response.message || 'Initialization failed')
          clearInterval(statusInterval)
        }
      } catch (error) {
        console.error('Failed to poll app status:', error)
      }
    }, 2000)

    return () => clearInterval(statusInterval)
  }, [])

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
            startPollingJob(runningJob.id)
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
        <div style={{ display: 'flex', alignItems: 'center', gap: '1rem' }}>
          <StatusBadge status={status} message={statusMessage} />
          {status === 'error' && (
            <button
              type="button"
              onClick={async () => {
                try {
                  await retryInit()
                  setStatus('initializing')
                  setStatusMessage('Retrying initialization...')
                } catch (error) {
                  console.error('Failed to retry:', error)
                }
              }}
              style={{
                padding: '0.5rem 1rem',
                background: 'var(--primary)',
                color: 'white',
                border: 'none',
                borderRadius: '6px',
                cursor: 'pointer',
                fontSize: '0.9rem',
                fontWeight: '500',
              }}
            >
              Retry
            </button>
          )}
        </div>
      </header>

      <div className="layout">
        <Sidebar currentView={currentView} onChangeView={setCurrentView} />

        <main className="main">
          {currentView === 'sources' && (
            selectedSourceId ? (
              <SourceProfile sourceId={selectedSourceId} onBack={() => setSelectedSourceId(null)} />
            ) : (
              <SourcesView
                onIndexResource={handleIndexResource}
                indexingResourceId={indexingResourceId}
                indexingProgress={indexingProgress}
                onCancelJob={handleCancelJob}
                onViewProfile={(sourceId) => setSelectedSourceId(sourceId)}
              />
            )
          )}
          {currentView === 'activity' && <ActivityView jobs={jobs} />}
          {currentView === 'assistant' && <AssistantView />}
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
    { id: 'assistant', label: 'AI Assistant', description: 'Test intent & enhancement' },
    { id: 'sources', label: 'Sources', description: 'Your knowledge sources' },
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
  message?: string | null
}

function StatusBadge({ status, message }: StatusBadgeProps) {
  let text = 'Idle'
  let className = 'status-pill idle'

  if (status === 'initializing') {
    text = message || 'Initializing'
    className = 'status-pill initializing'
  } else if (status === 'indexing') {
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
  onViewProfile: (sourceId: string) => void
}

function SourcesView({ onIndexResource, indexingResourceId, indexingProgress, onCancelJob, onViewProfile }: SourcesViewProps) {
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
          onViewProfile={onViewProfile}
        />
      </section>
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

function AssistantView() {
  const [query, setQuery] = useState('')
  const [strategy, setStrategy] = useState<PromptStrategy>('full_code')

  // Unified state
  const [processing, setProcessing] = useState(false)
  const [result, setResult] = useState<{
    original_query: string;
    enhanced_prompt: string;
    intent: IntentType;
    context_chunks: string[];
    preferences_applied: boolean;
  } | null>(null)
  const [error, setError] = useState('')
  const [copied, setCopied] = useState(false)
  const [showDetails, setShowDetails] = useState(false)

  // Preferences state
  const [preferences, setPreferences] = useState<UserPreferences>({})
  const [loadingPrefs, setLoadingPrefs] = useState(false)
  const [savingPrefs, setSavingPrefs] = useState(false)
  const [prefsStatus, setPrefsStatus] = useState('')

  useEffect(() => {
    loadPreferences()
  }, [])

  const loadPreferences = async () => {
    setLoadingPrefs(true)
    try {
      const response = await getPreferences()
      setPreferences(response.preferences)
    } catch (error) {
      console.error('Failed to load preferences:', error)
    } finally {
      setLoadingPrefs(false)
    }
  }

  const handleSavePreferences = async () => {
    setSavingPrefs(true)
    setPrefsStatus('')
    try {
      await updatePreferences(preferences)
      setPrefsStatus('✓ Preferences saved')
      setTimeout(() => setPrefsStatus(''), 3000)
    } catch (error) {
      setPrefsStatus(`✗ Failed to save: ${error}`)
    } finally {
      setSavingPrefs(false)
    }
  }

  const handleEnhance = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!query.trim()) return

    setProcessing(true)
    setError('')
    setResult(null)
    setCopied(false)
    setShowDetails(false)

    try {
      // Direct call to enhancePrompt which handles intent + enhancement
      const enhanced = await enhancePrompt(query, strategy)
      setResult(enhanced)
    } catch (error) {
      setError(`${error}`)
    } finally {
      setProcessing(false)
    }
  }

  const handleCopy = async () => {
    if (!result) return
    try {
      await navigator.clipboard.writeText(result.enhanced_prompt)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch (err) {
      console.error('Failed to copy:', err)
    }
  }

  const formatIntent = (intent: IntentType): string => {
    if (typeof intent === 'string') {
      return intent.replace(/_/g, ' ').replace(/\b\w/g, l => l.toUpperCase())
    } else if (typeof intent === 'object' && 'other' in intent) {
      return `Other: ${intent.other}`
    }
    return String(intent)
  }

  return (
    <div className="view">
      <div className="view-header">
        <h2>🤖 AI Assistant</h2>
        <p>Enhance your queries with context and preferences for better AI results.</p>
      </div>

      <div className="assistant-layout">
        <div className="assistant-main-col">
          {/* Query Input */}
          <section className="section">
            <form onSubmit={handleEnhance}>
              <div className="form-group">
                <label htmlFor="query">Your Query</label>
                <textarea
                  id="query"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder="e.g., 'Fix the timeout bug in auth service' or 'Explain how the login function works'"
                  rows={3}
                  required
                />
              </div>
              <div className="form-group">
                <label htmlFor="strategy">Prompt Strategy</label>
                <select
                  id="strategy"
                  value={strategy}
                  onChange={(e) => setStrategy(e.target.value as PromptStrategy)}
                >
                  <option value="full_code">Full Code (Default)</option>
                  <option value="reference_only">Reference Only</option>
                  <option value="architectural">Architectural</option>
                </select>
              </div>
              <button type="submit" disabled={processing}>
                {processing ? '✨ Enhancing...' : '✨ Enhance Prompt'}
              </button>
            </form>
          </section>

          {/* Results Area */}
          {error && <div className="status error">{error}</div>}

          {result && (
            <section className="section">
              <div className="result-header-row">
                <h3>Enhanced Prompt</h3>
                <div className="result-badges">
                  <span className="badge intent-badge">
                    🎯 {formatIntent(result.intent)}
                  </span>
                  <span className="badge context-badge">
                    📚 {result.context_chunks ? result.context_chunks.length : 0} Chunks
                  </span>
                </div>
              </div>

              <div className="enhanced-prompt-container">
                <div className="prompt-preview">
                  {result.enhanced_prompt.length > 300 && !showDetails
                    ? result.enhanced_prompt.slice(0, 300) + '...'
                    : result.enhanced_prompt}
                </div>
                <button
                  type="button"
                  className={`copy-btn ${copied ? 'copied' : ''}`}
                  onClick={handleCopy}
                >
                  {copied ? '✓ Copied!' : '📋 Copy Full Prompt'}
                </button>
              </div>

              <div className="details-toggle">
                <button
                  type="button"
                  className="text-btn"
                  onClick={() => setShowDetails(!showDetails)}
                >
                  {showDetails ? 'Hide Details ▲' : 'Show Details & Context ▼'}
                </button>
              </div>

              {showDetails && (
                <div className="details-panel">
                  <div className="detail-group">
                    <h4>Original Query</h4>
                    <div className="code-block">{result.original_query}</div>
                  </div>

                  <div className="detail-group">
                    <h4>Retrieved Context ({result.context_chunks ? result.context_chunks.length : 0})</h4>
                    {result.context_chunks && result.context_chunks.map((chunk, i) => (
                      <div key={i} className="context-chunk-preview">
                        <div className="chunk-label">Chunk {i + 1}</div>
                        <div className="code-block small">{chunk}</div>
                      </div>
                    ))}
                  </div>

                  <div className="detail-group">
                    <h4>Full Enhanced Prompt</h4>
                    <div className="code-block">{result.enhanced_prompt}</div>
                  </div>
                </div>
              )}
            </section>
          )}

          {!result && !processing && !error && (
            <div className="empty-state">
              Enter a query above to get an optimized prompt with context.
            </div>
          )}
        </div>

        <div className="assistant-sidebar-col">
          {/* User Preferences */}
          <section className="section">
            <h3>⚙️ User Preferences</h3>

            <p className="section-description">Configure your preferences to customize prompt enhancement.</p>

            {loadingPrefs ? (
              <div className="loading">Loading preferences...</div>
            ) : (
              <div className="preferences-form">
                <div className="form-group">
                  <label htmlFor="explanation_style">Explanation Style</label>
                  <input
                    id="explanation_style"
                    type="text"
                    value={preferences.explanation_style || ''}
                    onChange={(e) => setPreferences({ ...preferences, explanation_style: e.target.value })}
                    placeholder="e.g., concise, detailed, bullet points"
                  />
                </div>

                <div className="form-group">
                  <label htmlFor="code_style">Code Style</label>
                  <input
                    id="code_style"
                    type="text"
                    value={preferences.code_style || ''}
                    onChange={(e) => setPreferences({ ...preferences, code_style: e.target.value })}
                    placeholder="e.g., functional, OOP, minimal"
                  />
                </div>

                <div className="form-group">
                  <label htmlFor="documentation_style">Documentation Style</label>
                  <input
                    id="documentation_style"
                    type="text"
                    value={preferences.documentation_style || ''}
                    onChange={(e) => setPreferences({ ...preferences, documentation_style: e.target.value })}
                    placeholder="e.g., JSDoc, inline comments, README"
                  />
                </div>

                <div className="form-group">
                  <label htmlFor="test_style">Test Style</label>
                  <input
                    id="test_style"
                    type="text"
                    value={preferences.test_style || ''}
                    onChange={(e) => setPreferences({ ...preferences, test_style: e.target.value })}
                    placeholder="e.g., unit tests, integration tests, TDD"
                  />
                </div>

                <div className="form-group">
                  <label htmlFor="language_preference">Language Preference</label>
                  <input
                    id="language_preference"
                    type="text"
                    value={preferences.language_preference || ''}
                    onChange={(e) => setPreferences({ ...preferences, language_preference: e.target.value })}
                    placeholder="e.g., Rust, TypeScript, Python"
                  />
                </div>

                <div className="form-group">
                  <label htmlFor="verbosity">Verbosity</label>
                  <select
                    id="verbosity"
                    value={preferences.verbosity || 'balanced'}
                    onChange={(e) => setPreferences({ ...preferences, verbosity: e.target.value })}
                  >
                    <option value="concise">Concise</option>
                    <option value="balanced">Balanced</option>
                    <option value="detailed">Detailed</option>
                  </select>
                </div>

                <button type="button" onClick={handleSavePreferences} disabled={savingPrefs}>
                  {savingPrefs ? 'Saving...' : 'Save Preferences'}
                </button>

                {prefsStatus && (
                  <div className={`status ${prefsStatus.startsWith('✓') ? 'success' : 'error'}`}>
                    {prefsStatus}
                  </div>
                )}
              </div>
            )}
          </section>
        </div>
      </div>
    </div >
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
