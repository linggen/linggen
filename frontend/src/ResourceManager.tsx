import { useState, useEffect } from 'react'
import { addResource, listResources, removeResource, type Resource, type ResourceType } from './api'

interface ResourceManagerProps {
  onIndexResource?: (resource: Resource) => void
  indexingResourceId?: string | null
  indexingProgress?: string | null
}

export function ResourceManager({ onIndexResource, indexingResourceId, indexingProgress }: ResourceManagerProps) {
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
          <div className="resource-cards">
            {resources.map((resource) => (
              <div key={resource.id} className="resource-card">
                <div className="resource-header">
                  <span className="resource-icon">{getResourceIcon(resource.resource_type)}</span>
                  <div className="resource-info">
                    <h4>{resource.name}</h4>
                    <span className="resource-type">{resource.resource_type}</span>
                  </div>
                  <div className="resource-actions">
                    {resource.resource_type === 'local' && (
                      <>
                        {indexingResourceId === resource.id ? (
                          <div className="indexing-status">
                            <span className="spinner">⏳</span>
                            <span className="indexing-text">
                              {indexingProgress || 'Indexing...'}
                            </span>
                          </div>
                        ) : (
                          <button
                            type="button"
                            className="secondary-button"
                            onClick={() => onIndexResource?.(resource)}
                            disabled={!onIndexResource || indexingResourceId !== null}
                          >
                            Index now
                          </button>
                        )}
                      </>
                    )}
                    <button
                      type="button"
                      className="remove-btn"
                      onClick={() => handleRemove(resource.id, resource.name)}
                      title="Remove resource"
                      disabled={indexingResourceId === resource.id}
                    >
                      ✕
                    </button>
                  </div>
                </div>
                <div className="resource-path">{resource.path}</div>
                <div className="resource-status">
                  <span className={`status-badge ${resource.enabled ? 'enabled' : 'disabled'}`}>
                    {resource.enabled ? '● Active' : '○ Inactive'}
                  </span>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

