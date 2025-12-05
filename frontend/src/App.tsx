import { useState, useEffect, useRef, useCallback } from 'react'
import './App.css'
import {
  indexSource,
  listJobs,
  listResources,
  addResource,
  cancelJob,
  getAppStatus,
  retryInit,
  renameResource,
  updateResourcePatterns,
  type Resource,
  type ResourceType,
} from './api'
import { WorkspaceView } from './views/WorkspaceView'
import { SourcesView } from './views/SourcesView'
import { AddSourceModal } from './components/AddSourceModal'
import { ActivityView } from './views/ActivityView'
import { AssistantView } from './views/AssistantView'
import { SettingsView } from './views/SettingsView'
import { ArchitectureView } from './views/ArchitectureView'
import { MainLayout } from './components/MainLayout'
import type { View } from './components/Sidebar'

// Core flows:
// 1) Manage sources (git/local/web) via Sidebar
// 2) Index content from sources (currently local folders) into LanceDB
// 3) AI Assistant for intent classification and prompt enhancement

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
  const [currentView, setCurrentView] = useState<View>('sources')
  const [status, setStatus] = useState<AppStatus>('initializing')
  const [statusMessage, setStatusMessage] = useState<string | null>(null)

  // Data State
  const [resources, setResources] = useState<Resource[]>([])
  const [jobs, setJobs] = useState<Job[]>([])

  // Selection State
  const [selectedSourceId, setSelectedSourceId] = useState<string | null>(null)
  const [selectedNotePath, setSelectedNotePath] = useState<string | null>(null)

  const handleSelectSource = (id: string | null) => {
    setSelectedSourceId(id)
    setSelectedNotePath(null)
  }

  const handleSelectNote = (sourceId: string, path: string) => {
    setSelectedSourceId(sourceId)
    setSelectedNotePath(path)
  }


  // Indexing State
  const [indexingResourceId, setIndexingResourceId] = useState<string | null>(null)
  const [indexingProgress, setIndexingProgress] = useState<string | null>(null)
  const [currentJobId, setCurrentJobId] = useState<string | null>(null)

  // Utils
  const [resourcesVersion, setResourcesVersion] = useState(0) // bump to refresh sources list

  // Modal State
  const [isAddSourceModalOpen, setIsAddSourceModalOpen] = useState(false)

  // Use ref to track polling interval and cancelling state
  const pollingIntervalRef = useRef<number | null>(null)
  const pollingTimeoutRef = useRef<number | null>(null)
  const isCancellingRef = useRef<boolean>(false)

  // --- Resource Management (Lifted from ResourceManager) ---

  const loadResources = useCallback(async () => {
    try {
      const response = await listResources()
      setResources(response.resources)
    } catch (err) {
      console.error('Failed to load resources:', err)
    }
  }, [])

  useEffect(() => {
    loadResources()
  }, [loadResources, resourcesVersion])

  const handleAddResource = async (name: string, type: ResourceType, path: string, include?: string[], exclude?: string[]) => {
    try {
      await addResource({
        name,
        resource_type: type,
        path,
        include_patterns: include,
        exclude_patterns: exclude
      })
      // Refresh list
      const res = await listResources()
      setResources(res.resources)
      setIsAddSourceModalOpen(false)
    } catch (err) {
      console.error('Failed to add resource:', err)
      throw err // Re-throw to let modal handle error display
    }
  }

  const handleEditResource = async (id: string, name: string, include: string[], exclude: string[]) => {
    try {
      // 1. Rename if changed (we don't have previous resource here easily, so just call rename. 
      // Optimally we'd check, but calling rename with same name is likely fine or we can fetch resource first)

      // Actually, we can just call the endpoints.
      await renameResource(id, name);
      // 2. Update patterns
      await updateResourcePatterns(id, include, exclude);

      // Refresh list
      const res = await listResources()
      setResources(res.resources)
    } catch (err) {
      console.error('Failed to edit resource:', err)
      throw err
    }
  }

  // --- Job Polling & Status ---

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

          // Trigger a refresh of source stats/details (with delay to ensure backend saves stats)
          setTimeout(() => {
            console.log('Refreshing resources after job completion...')
            setResourcesVersion((v) => v + 1)
          }, 2000)

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

          // Trigger a refresh of source stats/details (with delay)
          setTimeout(() => {
            console.log('Refreshing resources after job failure...')
            setResourcesVersion((v) => v + 1)
          }, 2000)

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
    console.log('Starting polling for job:', jobId)

    // Clear any existing polling
    if (pollingIntervalRef.current) {
      clearInterval(pollingIntervalRef.current)
    }
    if (pollingTimeoutRef.current) {
      clearTimeout(pollingTimeoutRef.current)
    }

    // Call immediately, then poll every second
    updateJobProgress(jobId)
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
    // ... (same as before, snipped for brevity if unchanged logic is trusted, assuming copied correctly)
    // Re-implementing logic to be safe since previous was read-only
    console.log('Cancel button clicked!')

    let jobIdToCancel = currentJobId
    if (!jobIdToCancel && indexingResourceId) {
      const runningJob = jobs.find(
        (job) => job.sourceId === indexingResourceId && job.status === 'running'
      )
      if (runningJob) jobIdToCancel = runningJob.id
    }

    if (!jobIdToCancel) {
      setIndexingProgress('✗ No active job found')
      return
    }

    isCancellingRef.current = true
    setIndexingProgress('Cancelling...')

    try {
      await cancelJob(jobIdToCancel)

      setTimeout(async () => {
        try {
          const response = await listJobs()
          const job = response.jobs.find((j) => j.id === jobIdToCancel)

          if (job && job.status === 'Failed' && job.error?.includes('cancelled')) {
            setIndexingProgress('✓ Cancelled')
            setTimeout(() => {
              setIndexingResourceId(null)
              setIndexingProgress(null)
              setCurrentJobId(null)
              setStatus('idle')
            }, 2000)
          } else if (job && job.status !== 'Running') {
            setIndexingProgress(`Job already ${job.status}`)
            setTimeout(() => {
              setIndexingResourceId(null)
              setIndexingProgress(null)
              setCurrentJobId(null)
              setStatus('idle')
            }, 2000)
          }
        } catch (error) {
          console.error(error)
        }
      }, 1000)
    } catch (error) {
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

  // Define Status Bar Element
  const renderStatusElement = () => {
    let statusText = 'Idle'
    let statusDotClass = 'status-dot idle'

    if (status === 'initializing') {
      statusText = statusMessage || 'Initializing...'
      statusDotClass = 'status-dot initializing'
    } else if (status === 'indexing') {
      statusText = 'Indexing...'
      statusDotClass = 'status-dot indexing'
    } else if (status === 'error') {
      statusText = 'Error'
      statusDotClass = 'status-dot error'
    }

    return (
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', width: '100%', padding: '0 8px' }}>
        <div className="status-item" style={{ display: 'flex', alignItems: 'center', gap: '8px', fontSize: '11px', color: 'var(--text-muted)' }}>
          <span className={statusDotClass} style={{ width: '6px', height: '6px', borderRadius: '50%', background: 'currentColor' }}></span>
          <span>{statusText}</span>
        </div>
        {status === 'error' && (
          <button
            onClick={handleRetryInit}
            style={{
              padding: '2px 8px',
              background: 'var(--error)',
              color: 'white',
              border: 'none',
              borderRadius: '4px',
              cursor: 'pointer',
              fontSize: '10px',
            }}
          >
            Retry
          </button>
        )}
      </div>
    )
  }

  return (
    <MainLayout
      currentView={currentView}
      onChangeView={setCurrentView}
      resources={resources}
      selectedSourceId={selectedSourceId}
      onSelectSource={handleSelectSource}
      selectedNotePath={selectedNotePath}
      onSelectNote={handleSelectNote}
      onAddSource={() => setIsAddSourceModalOpen(true)}
      statusElement={renderStatusElement()}
    >
      {currentView === 'sources' && (
        selectedSourceId ? (
          <WorkspaceView
            sourceId={selectedSourceId}
            source={resources.find(r => r.id === selectedSourceId)}
            onIndexComplete={() => setResourcesVersion(v => v + 1)}
            onIndexResource={handleIndexResource}
            indexingResourceId={indexingResourceId}
            indexingProgress={indexingProgress}
            onUpdateSource={handleEditResource}
            selectedNotePath={selectedNotePath}
          />
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
      {currentView === 'architecture' && <ArchitectureView />}
      {currentView === 'activity' && <ActivityView jobs={jobs} />}
      {currentView === 'assistant' && <AssistantView />}
      {currentView === 'settings' && <SettingsView />}

      <AddSourceModal
        isOpen={isAddSourceModalOpen}
        onClose={() => setIsAddSourceModalOpen(false)}
        onAdd={handleAddResource}
      />
    </MainLayout>
  )
}

export default App
