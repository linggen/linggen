import { useState, useEffect, useRef, useCallback } from 'react'
import './App.css'
import {
  indexSource,
  listJobs,
  cancelJob,
  getAppStatus,
  retryInit,
  type Resource,
  type ResourceType,
} from './api'
import { SourceProfile } from './components/ProjectProfile'
import { AppHeader } from './components/AppHeader'
import { SourcesView } from './views/SourcesView'
import { ActivityView } from './views/ActivityView'
import { AssistantView } from './views/AssistantView'
import { SettingsView } from './views/SettingsView'

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
  const [resourcesVersion, setResourcesVersion] = useState(0) // bump to refresh sources list

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

          // Trigger a refresh of source stats/details
          setResourcesVersion((v) => v + 1)

          setTimeout(() => {
            setIndexingResourceId(null)
            setIndexingProgress(null)
            setCurrentJobId(null)
            setStatus('idle')
          }, 3000)
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

          // Trigger a refresh of source stats/details as well
          setResourcesVersion((v) => v + 1)

          setTimeout(() => {
            setIndexingResourceId(null)
            setIndexingProgress(null)
            setCurrentJobId(null)
            isCancellingRef.current = false
            setStatus(errorMsg.includes('cancelled') ? 'idle' : 'error')
          }, 3000)
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

        // Check if any job is still running or pending
        const hasActiveJob = backendJobs.some((job) => job.status === 'running' || job.status === 'pending')
        if (hasActiveJob) {
          setStatus('indexing')
          // Find the running/pending job and set indexing state
          const activeJob = backendJobs.find((job) => job.status === 'running' || job.status === 'pending')
          if (activeJob?.sourceId) {
            setIndexingResourceId(activeJob.sourceId)
            setCurrentJobId(activeJob.id) // Set the job ID for cancellation
            setIndexingProgress(activeJob.status === 'pending' ? '⏳ Waiting in queue...' : 'Indexing in progress...')
            startPollingJob(activeJob.id)
          }
        } else {
          // No active jobs, ensure we're in idle state
          setStatus('idle')
          setIndexingResourceId(null)
          setIndexingProgress(null)
          setCurrentJobId(null)
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

  const handleRetryInit = async () => {
    try {
      await retryInit()
      setStatus('initializing')
      setStatusMessage('Retrying initialization...')
    } catch (error) {
      console.error('Failed to retry:', error)
    }
  }

  return (
    <div className="app">
      <AppHeader
        status={status}
        message={statusMessage}
        onRetry={handleRetryInit}
      />

      <TopNav currentView={currentView} onChangeView={setCurrentView} />

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
              resourcesVersion={resourcesVersion}
            />
          )
        )}
        {currentView === 'activity' && <ActivityView jobs={jobs} />}
        {currentView === 'assistant' && <AssistantView />}
        {currentView === 'settings' && <SettingsView />}
      </main>
    </div>
  )
}

interface TopNavProps {
  currentView: View
  onChangeView: (view: View) => void
}

function TopNav({ currentView, onChangeView }: TopNavProps) {
  const items: { id: View; label: string }[] = [
    { id: 'assistant', label: 'AI Assistant' },
    { id: 'sources', label: 'Sources' },
    { id: 'activity', label: 'Activity' },
    { id: 'settings', label: 'Settings' },
  ]

  return (
    <nav className="top-nav">
      <ul className="top-nav-list">
        {items.map((item) => (
          <li key={item.id}>
            <button
              type="button"
              className={`top-nav-item ${currentView === item.id ? 'active' : ''}`}
              onClick={() => onChangeView(item.id)}
            >
              {item.label}
            </button>
          </li>
        ))}
      </ul>
    </nav>
  )
}

export default App
