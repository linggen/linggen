import { useState, useEffect, useRef } from 'react'
import { addResource, listResources, removeResource, renameResource, uploadFile, type Resource, type ResourceType, type IndexMode } from './api'

interface ResourceManagerProps {
  onIndexResource?: (resource: Resource, mode?: IndexMode) => void
  indexingResourceId?: string | null
  indexingProgress?: string | null
  onCancelJob?: () => void
  onViewProfile?: (sourceId: string) => void
  refreshKey?: number
}

export function ResourceManager({
  onIndexResource,
  indexingResourceId,
  indexingProgress,
  onCancelJob,
  onViewProfile,
  refreshKey,
}: ResourceManagerProps) {
  const [resources, setResources] = useState<Resource[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [success, setSuccess] = useState('')

  // Form state
  const [name, setName] = useState('')
  const [resourceType, setResourceType] = useState<ResourceType>('local')
  const [path, setPath] = useState('')
  const [includePatterns, setIncludePatterns] = useState('')
  const [excludePatterns, setExcludePatterns] = useState('')
  const [showAdvanced, setShowAdvanced] = useState(false)
  const [adding, setAdding] = useState(false)

  // Rename state
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editName, setEditName] = useState('')
  const editInputRef = useRef<HTMLInputElement>(null)

  // Upload state
  const [uploadingSourceId, setUploadingSourceId] = useState<string | null>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)

  // Remove confirmation modal state
  const [removeConfirm, setRemoveConfirm] = useState<{ id: string; name: string } | null>(null)
  const [removing, setRemoving] = useState(false)

  // Track whether component is mounted to avoid setting state after unmount
  const isMountedRef = useRef(true)

  useEffect(() => {
    isMountedRef.current = true
    const controller = new AbortController()

    // Retry a few times on startup because the backend may still be initializing
    const loadWithRetry = async (attempt = 1) => {
      try {
        if (!isMountedRef.current) return
        setLoading(true)
        setError('')
        const response = await listResources()
        if (!isMountedRef.current) return
        setResources(response.resources)
      } catch (err) {
        console.error('Failed to load resources (attempt', attempt, '):', err)
        if (!isMountedRef.current) return

        if (attempt < 5) {
          // Backoff a bit between retries
          setTimeout(() => loadWithRetry(attempt + 1), 500 * attempt)
        } else {
          setError(`Failed to load resources: ${err}`)
        }
      } finally {
        if (isMountedRef.current) {
          setLoading(false)
        }
      }
    }

    loadWithRetry()

    // Re-load whenever parent indicates resources have changed (e.g., after indexing)
    return () => {
      isMountedRef.current = false
      controller.abort()
    }
  }, [refreshKey])

  const loadResources = async () => {
    setLoading(true)
    setError('')
    try {
      const response = await listResources()
      setResources(response.resources)
    } catch (err) {
      setError(`Failed to load resources: ${err}`)
    } finally {
      setLoading(false)
    }
  }

  const handleAdd = async (e: React.FormEvent) => {
    e.preventDefault()
    setAdding(true)
    setError('')
    setSuccess('')

    try {
      // Parse comma-separated patterns into arrays
      const includePatternsArray = includePatterns
        .split(',')
        .map(p => p.trim())
        .filter(p => p.length > 0)
      const excludePatternsArray = excludePatterns
        .split(',')
        .map(p => p.trim())
        .filter(p => p.length > 0)

      const newResource = await addResource({
        name,
        resource_type: resourceType,
        path,
        include_patterns: includePatternsArray.length > 0 ? includePatternsArray : undefined,
        exclude_patterns: excludePatternsArray.length > 0 ? excludePatternsArray : undefined,
      })
      setSuccess(`✓ Added resource: ${name}`)
      setName('')
      setPath('')
      setIncludePatterns('')
      setExcludePatterns('')
      setShowAdvanced(false)
      await loadResources()

      // For uploads type, navigate to detail page to upload files
      if (resourceType === 'uploads' && newResource.id) {
        onViewProfile?.(newResource.id)
      }
    } catch (err) {
      setError(`✗ Failed to add resource: ${err}`)
    } finally {
      setAdding(false)
    }
  }

  const handleRemove = (id: string, name: string) => {
    setRemoveConfirm({ id, name })
  }

  const confirmRemove = async () => {
    if (!removeConfirm) return

    setRemoving(true)
    setError('')
    setSuccess('')

    try {
      await removeResource(removeConfirm.id)
      setSuccess(`✓ Removed resource: ${removeConfirm.name}`)
      setRemoveConfirm(null)
      await loadResources()
    } catch (err) {
      setError(`✗ Failed to remove resource: ${err}`)
    } finally {
      setRemoving(false)
    }
  }

  const startRename = (resource: Resource) => {
    setEditingId(resource.id)
    setEditName(resource.name)
    // Focus input after render
    setTimeout(() => editInputRef.current?.focus(), 0)
  }

  const handleRename = async (id: string) => {
    if (!editName.trim()) {
      setEditingId(null)
      return
    }

    setError('')
    setSuccess('')

    try {
      await renameResource(id, editName.trim())
      setSuccess(`✓ Renamed resource to: ${editName.trim()}`)
      setEditingId(null)
      await loadResources()
    } catch (err) {
      setError(`✗ Failed to rename resource: ${err}`)
    }
  }

  const cancelRename = () => {
    setEditingId(null)
    setEditName('')
  }

  const handleUploadClick = (sourceId: string) => {
    setUploadingSourceId(sourceId)

    // Listen for window focus to detect when file dialog is closed without selection
    // This handles the case where user cancels the file picker
    const handleWindowFocus = () => {
      // Small delay to allow onChange to fire first if a file was selected
      setTimeout(() => {
        if (fileInputRef.current && fileInputRef.current.files?.length === 0) {
          setUploadingSourceId(null)
        }
      }, 300)
      window.removeEventListener('focus', handleWindowFocus)
    }
    window.addEventListener('focus', handleWindowFocus)

    fileInputRef.current?.click()
  }

  const handleFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files
    if (!files || files.length === 0 || !uploadingSourceId) {
      setUploadingSourceId(null)
      return
    }

    setError('')
    setSuccess('')

    try {
      for (const file of Array.from(files)) {
        const result = await uploadFile(uploadingSourceId, file)
        setSuccess(`✓ Uploaded "${result.filename}": ${result.chunks_created} chunks created`)
      }
      await loadResources()
    } catch (err) {
      setError(`✗ Failed to upload: ${err}`)
    } finally {
      setUploadingSourceId(null)
      // Reset file input
      if (fileInputRef.current) {
        fileInputRef.current.value = ''
      }
    }
  }

  const getResourceIcon = (type: ResourceType) => {
    switch (type) {
      case 'git':
        return '🔗'
      case 'local':
        return '📁'
      case 'web':
        return '🌐'
      case 'uploads':
        return '📥'
      default:
        return '📄'
    }
  }

  const getPlaceholder = (type: ResourceType) => {
    switch (type) {
      case 'git':
        return 'https://github.com/user/repo.git'
      case 'local':
        return '/path/to/folder'
      case 'web':
        return 'https://docs.example.com'
      case 'uploads':
        return 'Managed by Linggen (folder created automatically)'
      default:
        return ''
    }
  }

  return (
    <div className="resource-manager">
      <h2>📚 Resource Management</h2>

      {/* Add Resource Form */}
      <form onSubmit={handleAdd} className="resource-form">
        <div className="form-row">
          <div className="form-group">
            <label htmlFor="name">Name</label>
            <input
              id="name"
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="My Documents"
              required
            />
          </div>

          <div className="form-group">
            <label htmlFor="type">Type</label>
            <select
              id="type"
              value={resourceType}
              onChange={(e) => {
                const newType = e.target.value as ResourceType
                setResourceType(newType)
                // Clear path when switching to uploads since it's managed
                if (newType === 'uploads') {
                  setPath('')
                }
              }}
            >
              <option value="local">Local Folder</option>
              <option value="uploads">Uploads</option>
              <option value="git" disabled>Git Repository (Coming Soon)</option>
              <option value="web" disabled>Website (Coming Soon)</option>
            </select>
          </div>
        </div>

        {resourceType === 'uploads' ? (
          <div className="uploads-hint">
            <span className="label-hint">
              Upload documents (PDF, DOCX, TXT, MD) directly. Click "Upload" on the source to add files.
            </span>
          </div>
        ) : (
          <>
            <div className="form-group">
              <label htmlFor="path">Path / URL</label>
              <input
                id="path"
                type="text"
                value={path}
                onChange={(e) => setPath(e.target.value)}
                placeholder={getPlaceholder(resourceType)}
                required
              />
            </div>

            {/* Advanced options toggle */}
            <button
              type="button"
              className="btn-toggle-advanced"
              onClick={() => setShowAdvanced(!showAdvanced)}
            >
              {showAdvanced ? '▼ Hide Filters' : '▶ File Filters (optional)'}
            </button>

            {showAdvanced && (
              <div className="advanced-options">
                <div className="form-group">
                  <label htmlFor="includePatterns">
                    Include Patterns
                    <span className="label-hint">Only index files matching these patterns</span>
                  </label>
                  <input
                    id="includePatterns"
                    type="text"
                    value={includePatterns}
                    onChange={(e) => setIncludePatterns(e.target.value)}
                    placeholder="*.cs, *.md, *.json"
                  />
                </div>

                <div className="form-group">
                  <label htmlFor="excludePatterns">
                    Exclude Patterns
                    <span className="label-hint">Skip files matching these patterns</span>
                  </label>
                  <input
                    id="excludePatterns"
                    type="text"
                    value={excludePatterns}
                    onChange={(e) => setExcludePatterns(e.target.value)}
                    placeholder="*.meta, *.asset, *.prefab"
                  />
                </div>

                <div className="pattern-examples">
                  <strong>Examples:</strong> <code>*.cs</code> (C# files), <code>*.md</code> (Markdown), <code>src/*.ts</code> (TypeScript in src)
                </div>
              </div>
            )}
          </>
        )}

        <div className="form-actions">
          <button type="submit" className="btn-action btn-index" disabled={adding}>
            {adding ? 'Adding...' : '+ Add Resource'}
          </button>
        </div>
      </form>

      {error && <div className="status error">{error}</div>}
      {success && <div className="status success">{success}</div>}

      {/* Hidden file input for uploads */}
      <input
        ref={fileInputRef}
        type="file"
        multiple
        accept=".pdf,.docx,.doc,.txt,.md,.markdown,.json,.yaml,.yml,.toml,.csv,.xml,.html,.htm,.rst,.tex"
        style={{ display: 'none' }}
        onChange={handleFileChange}
      />

      {/* Resources List */}
      <div className="resources-list">
        <h3>Configured Resources ({resources.length})</h3>

        {loading ? (
          <div className="loading">Loading resources...</div>
        ) : (
          <div className="resource-table">
            <div className="resource-table-header">
              <div className="col-name">Resource</div>
              <div className="col-location">Location</div>
              <div className="col-stats">Stats</div>
              <div className="col-status">Last Indexed</div>
              <div className="col-actions">Actions</div>
            </div>

            {resources.length === 0 ? (
              <div className="empty-state">
                No resources configured yet. Add one above to get started!
              </div>
            ) : (
              resources.map((resource) => (
                <div key={resource.id} className="resource-row">
                  <div className="col-name">
                    <div className="resource-name-cell">
                      <span className="resource-icon">{getResourceIcon(resource.resource_type)}</span>
                      <div className="resource-details">
                        {editingId === resource.id ? (
                          <div className="rename-input-container">
                            <input
                              ref={editInputRef}
                              type="text"
                              className="rename-input"
                              value={editName}
                              onChange={(e) => setEditName(e.target.value)}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  handleRename(resource.id)
                                } else if (e.key === 'Escape') {
                                  cancelRename()
                                }
                              }}
                              onBlur={() => handleRename(resource.id)}
                            />
                          </div>
                        ) : (
                          <a
                            href="#"
                            className="resource-title-link"
                            onClick={(e) => {
                              e.preventDefault();
                              onViewProfile?.(resource.id);
                            }}
                            title="View profile"
                          >
                            {resource.name}
                          </a>
                        )}
                        <div className="resource-type-badge">{resource.resource_type.toUpperCase()}</div>
                      </div>
                    </div>
                  </div>
                  <div className="col-location">
                    {resource.resource_type === 'uploads' ? (
                      <span className="uploads-location-hint">Direct upload</span>
                    ) : (
                      <span className="resource-path-text">{resource.path}</span>
                    )}
                  </div>
                  <div className="col-stats">
                    {resource.stats ? (
                      <div className="stats-cell">
                        <div className="stat-item" title="Files">
                          <span className="stat-value">{resource.stats.file_count.toLocaleString()}</span>
                          <span className="stat-label">files</span>
                        </div>
                        <div className="stat-item" title="Chunks">
                          <span className="stat-value">{resource.stats.chunk_count.toLocaleString()}</span>
                          <span className="stat-label">chunks</span>
                        </div>
                        <div className="stat-item" title="Size">
                          <span className="stat-value">{(resource.stats.total_size_bytes / 1024).toFixed(0)}</span>
                          <span className="stat-label">KB</span>
                        </div>
                      </div>
                    ) : (
                      <span className="stats-empty">-</span>
                    )}
                  </div>
                  <div className="col-status">
                    {indexingResourceId === resource.id ? (
                      <div className="indexing-indicator">
                        <span className="spinner">⏳</span>
                        <span className="indexing-label">{indexingProgress || 'Indexing...'}</span>
                      </div>
                    ) : resource.latest_job ? (
                      <div className="status-cell">
                        {resource.latest_job.status === 'Completed' && (
                          <div className="status-completed">
                            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" xmlns="http://www.w3.org/2000/svg">
                              <circle cx="7" cy="7" r="6" stroke="currentColor" strokeWidth="1.5" fill="none" />
                              <path d="M4 7L6 9L10 5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
                            </svg>
                            <span title={new Date(resource.latest_job.finished_at || '').toLocaleString()}>
                              {new Date(resource.latest_job.finished_at || '').toLocaleDateString()}
                            </span>
                          </div>
                        )}
                        {resource.latest_job.status === 'Failed' && (
                          <div className="status-failed" title={resource.latest_job.error}>
                            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" xmlns="http://www.w3.org/2000/svg">
                              <circle cx="7" cy="7" r="6" stroke="currentColor" strokeWidth="1.5" />
                              <path d="M5 5L9 9M9 5L5 9" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
                            </svg>
                            <span>Failed</span>
                          </div>
                        )}
                      </div>
                    ) : resource.resource_type === 'uploads' && resource.last_upload_time ? (
                      <div className="status-completed">
                        <svg width="14" height="14" viewBox="0 0 14 14" fill="none" xmlns="http://www.w3.org/2000/svg">
                          <circle cx="7" cy="7" r="6" stroke="currentColor" strokeWidth="1.5" fill="none" />
                          <path d="M4 7L6 9L10 5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
                        </svg>
                        <span title={new Date(resource.last_upload_time).toLocaleString()}>
                          {new Date(resource.last_upload_time).toLocaleDateString()}
                        </span>
                      </div>
                    ) : (
                      <span className="status-never">Never</span>
                    )}
                  </div>
                  <div className="col-actions">
                    <div className="action-buttons">
                      {resource.resource_type === 'uploads' ? (
                        <button
                          type="button"
                          className="btn-action btn-upload"
                          onClick={() => handleUploadClick(resource.id)}
                          disabled={uploadingSourceId === resource.id}
                        >
                          {uploadingSourceId === resource.id ? 'Uploading...' : 'Upload'}
                        </button>
                      ) : resource.resource_type === 'local' && (
                        <>
                          {indexingResourceId === resource.id ? (
                            <button
                              type="button"
                              className="btn-action btn-cancel"
                              onClick={onCancelJob}
                              title="Cancel indexing"
                            >
                              Cancel
                            </button>
                          ) : (
                            <button
                              type="button"
                              className="btn-action btn-index"
                              onClick={(e) => {
                                // Shift+click for full reindex, normal click for incremental
                                const mode = e.shiftKey ? 'full' : 'incremental';
                                onIndexResource?.(resource, mode);
                              }}
                              disabled={!onIndexResource || indexingResourceId === resource.id}
                              title={resource.latest_job?.status === 'Completed' 
                                ? 'Update changed files (Shift+Click for full reindex)' 
                                : 'Index all files'}
                            >
                              {resource.latest_job?.status === 'Completed' ? 'Update' : 'Index'}
                            </button>
                          )}
                        </>
                      )}
                      <button
                        type="button"
                        className="btn-action btn-rename"
                        onClick={() => startRename(resource)}
                        title="Rename resource"
                        disabled={indexingResourceId === resource.id || editingId !== null}
                      >
                        Rename
                      </button>
                      <button
                        type="button"
                        className="btn-action btn-remove"
                        onClick={() => handleRemove(resource.id, resource.name)}
                        title="Remove resource"
                        disabled={indexingResourceId === resource.id}
                      >
                        Remove
                      </button>
                    </div>
                  </div>
                </div>
              ))
            )}
          </div>
        )}
      </div>

      {/* Remove Confirmation Modal */}
      {removeConfirm && (
        <div className="modal-overlay" onClick={() => !removing && setRemoveConfirm(null)}>
          <div className="modal-content" onClick={e => e.stopPropagation()}>
            <div className="modal-header">
              <h3>🗑️ Remove Source</h3>
            </div>
            <div className="modal-body">
              <p style={{ marginBottom: '1rem', color: 'var(--text)' }}>
                Are you sure you want to remove <strong>"{removeConfirm.name}"</strong>?
              </p>
              <p style={{ color: 'var(--text-muted)', fontSize: '0.85rem', marginBottom: '0.5rem' }}>
                This will delete:
              </p>
              <ul style={{ margin: '0 0 1rem 1.5rem', color: 'var(--text-muted)', lineHeight: '1.8', fontSize: '0.85rem' }}>
                <li>All indexed chunks from the vector database</li>
                <li>Source configuration and profile</li>
              </ul>
              <p style={{ color: '#ef4444', fontWeight: '500', fontSize: '0.85rem' }}>
                This action cannot be undone.
              </p>
            </div>
            <div className="modal-footer">
              <button
                type="button"
                className="btn-secondary"
                onClick={() => setRemoveConfirm(null)}
                disabled={removing}
              >
                Cancel
              </button>
              <button
                type="button"
                className="btn-danger"
                onClick={confirmRemove}
                disabled={removing}
              >
                {removing ? 'Removing...' : '🗑️ Remove'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
