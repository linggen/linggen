import { useState, useEffect } from 'react'
import { addResource, listResources, removeResource, type Resource, type ResourceType } from './api'

interface ResourceManagerProps {
  onIndexResource?: (resource: Resource) => void
  indexingResourceId?: string | null
  indexingProgress?: string | null
  onCancelJob?: () => void
}

export function ResourceManager({ onIndexResource, indexingResourceId, onCancelJob }: ResourceManagerProps) {
  const [resources, setResources] = useState<Resource[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [success, setSuccess] = useState('')

  // Form state
  const [name, setName] = useState('')
  const [resourceType, setResourceType] = useState<ResourceType>('local')
  const [path, setPath] = useState('')
  const [adding, setAdding] = useState(false)

  useEffect(() => {
    loadResources()
  }, [])

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
      await addResource({
        name,
        resource_type: resourceType,
        path,
      })
      setSuccess(`✓ Added resource: ${name}`)
      setName('')
      setPath('')
      await loadResources()
    } catch (err) {
      setError(`✗ Failed to add resource: ${err}`)
    } finally {
      setAdding(false)
    }
  }

  const handleRemove = async (id: string, name: string) => {
    if (!confirm(`Are you sure you want to remove "${name}"?`)) {
      return
    }

    setError('')
    setSuccess('')

    try {
      await removeResource(id)
      setSuccess(`✓ Removed resource: ${name}`)
      await loadResources()
    } catch (err) {
      setError(`✗ Failed to remove resource: ${err}`)
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
              onChange={(e) => setResourceType(e.target.value as ResourceType)}
            >
              <option value="local">Local Folder</option>
              <option value="git">Git Repository</option>
              <option value="web">Website</option>
            </select>
          </div>
        </div>

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

        <button type="submit" disabled={adding}>
          {adding ? 'Adding...' : '+ Add Resource'}
        </button>
      </form>

      {error && <div className="status error">{error}</div>}
      {success && <div className="status success">{success}</div>}

      {/* Resources List */}
      <div className="resources-list">
        <h3>Configured Resources ({resources.length})</h3>

        {loading ? (
          <div className="loading">Loading resources...</div>
        ) : resources.length === 0 ? (
          <div className="empty-state">
            No resources configured yet. Add one above to get started!
          </div>
        ) : (
          <div className="resource-table">
            <div className="resource-table-header">
              <div className="col-name">Resource</div>
              <div className="col-location">Location</div>
              <div className="col-status">Last Indexed</div>
              <div className="col-actions">Actions</div>
            </div>
            {resources.map((resource) => (
              <div key={resource.id} className="resource-row">
                <div className="col-name">
                  <div className="resource-name-cell">
                    <span className="resource-icon">{getResourceIcon(resource.resource_type)}</span>
                    <div className="resource-details">
                      <div className="resource-title">{resource.name}</div>
                      <div className="resource-type-badge">{resource.resource_type.toUpperCase()}</div>
                    </div>
                  </div>
                </div>
                <div className="col-location">
                  <span className="resource-path-text">{resource.path}</span>
                </div>
                <div className="col-status">
                  {indexingResourceId === resource.id ? (
                    <div className="indexing-indicator">
                      <span className="spinner">⏳</span>
                      <span className="indexing-label">Indexing...</span>
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
                  ) : (
                    <span className="status-never">Never</span>
                  )}
                </div>
                <div className="col-actions">
                  <div className="action-buttons">
                    {resource.resource_type === 'local' && (
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
                            onClick={() => onIndexResource?.(resource)}
                            disabled={!onIndexResource || indexingResourceId !== null}
                          >
                            {resource.latest_job?.status === 'Completed' ? 'Update' : 'Index'}
                          </button>
                        )}
                      </>
                    )}
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
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

