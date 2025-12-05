import { useState, useEffect, useCallback } from 'react';
import { GraphView, type SelectedNodeInfo } from '../components/GraphView';
import { DesignEditor } from '../components/DesignEditor';
import { listResources, type Resource } from '../api';
import './ArchitectureView.css';

type CenterPane = 'graph' | 'editor';

export function ArchitectureView() {
  const [sources, setSources] = useState<Resource[]>([]);
  const [selectedSourceId, setSelectedSourceId] = useState<string | null>(null);
  const [centerPane, setCenterPane] = useState<CenterPane>('graph');
  const [selectedNote, setSelectedNote] = useState<string | null>(null);
  const [selectedNode, setSelectedNode] = useState<SelectedNodeInfo | null>(null);
  const [leftSidebarCollapsed, setLeftSidebarCollapsed] = useState(false);
  const [rightSidebarCollapsed, setRightSidebarCollapsed] = useState(true);
  const [loading, setLoading] = useState(true);
  const [focusNodeId, setFocusNodeId] = useState<string | null>(null);

  // Load sources
  useEffect(() => {
    const loadSources = async () => {
      try {
        const response = await listResources();
        // Only show local sources (they have graphs)
        const localSources = response.resources.filter(r => r.resource_type === 'local');
        setSources(localSources);
        
        // Auto-select first source if available
        if (localSources.length > 0 && !selectedSourceId) {
          setSelectedSourceId(localSources[0].id);
        }
      } catch (err) {
        console.error('Failed to load sources:', err);
      } finally {
        setLoading(false);
      }
    };
    
    loadSources();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Handle node selection from graph
  const handleNodeSelect = useCallback((node: SelectedNodeInfo | null) => {
    setSelectedNode(node);
    if (node) {
      setRightSidebarCollapsed(false);
    }
  }, []);

  // Handle opening design note for a node
  const handleOpenNoteForNode = useCallback((nodeId: string) => {
    // Convert file path to note path convention
    const notePath = `file/${nodeId.replace(/\//g, '-')}.md`;
    setSelectedNote(notePath);
    setCenterPane('editor');
  }, []);

  // Handle focusing on a node from the editor
  const handleFocusNode = useCallback((nodeId: string) => {
    setFocusNodeId(nodeId);
    setCenterPane('graph');
  }, []);

  // Get selected source
  const selectedSource = sources.find(s => s.id === selectedSourceId);

  if (loading) {
    return (
      <div className="architecture-view architecture-loading">
        <div className="loading-spinner"></div>
        <p>Loading sources...</p>
      </div>
    );
  }

  if (sources.length === 0) {
    return (
      <div className="architecture-view architecture-empty">
        <div className="empty-icon">🏗️</div>
        <h3>No local sources available</h3>
        <p>Add a local source from the Sources tab to start exploring your codebase architecture.</p>
      </div>
    );
  }

  return (
    <div className="architecture-view">
      {/* Left Sidebar - Sources & Notes */}
      <aside className={`architecture-sidebar architecture-sidebar-left ${leftSidebarCollapsed ? 'collapsed' : ''}`}>
        <button 
          className="sidebar-toggle sidebar-toggle-left"
          onClick={() => setLeftSidebarCollapsed(!leftSidebarCollapsed)}
          title={leftSidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
        >
          {leftSidebarCollapsed ? '→' : '←'}
        </button>
        
        {!leftSidebarCollapsed && (
          <div className="sidebar-content">
            {/* Source Selector */}
            <div className="sidebar-section">
              <h4 className="sidebar-section-title">Source</h4>
              <select
                value={selectedSourceId || ''}
                onChange={(e) => setSelectedSourceId(e.target.value)}
                className="source-select"
              >
                {sources.map((source) => (
                  <option key={source.id} value={source.id}>
                    {source.name}
                  </option>
                ))}
              </select>
              {selectedSource && (
                <div className="source-info">
                  <span className="source-path" title={selectedSource.path}>
                    {selectedSource.path}
                  </span>
                  {selectedSource.stats && (
                    <div className="source-stats">
                      <span>{selectedSource.stats.file_count} files</span>
                      <span>{selectedSource.stats.chunk_count} chunks</span>
                    </div>
                  )}
                </div>
              )}
            </div>

            {/* View Toggle */}
            <div className="sidebar-section">
              <h4 className="sidebar-section-title">View</h4>
              <div className="view-toggle">
                <button
                  className={`view-toggle-btn ${centerPane === 'graph' ? 'active' : ''}`}
                  onClick={() => setCenterPane('graph')}
                >
                  📊 Graph
                </button>
                <button
                  className={`view-toggle-btn ${centerPane === 'editor' ? 'active' : ''}`}
                  onClick={() => setCenterPane('editor')}
                >
                  📝 Design
                </button>
              </div>
            </div>

            {/* Quick Actions */}
            <div className="sidebar-section">
              <h4 className="sidebar-section-title">Quick Notes</h4>
              <div className="quick-notes">
                <button 
                  className="quick-note-btn"
                  onClick={() => {
                    setSelectedNote('system-overview.md');
                    setCenterPane('editor');
                  }}
                >
                  📋 System Overview
                </button>
                <button 
                  className="quick-note-btn"
                  onClick={() => {
                    setSelectedNote('architecture.md');
                    setCenterPane('editor');
                  }}
                >
                  🏛️ Architecture
                </button>
                <button 
                  className="quick-note-btn"
                  onClick={() => {
                    setSelectedNote('conventions.md');
                    setCenterPane('editor');
                  }}
                >
                  📏 Conventions
                </button>
              </div>
            </div>

            {/* LLM Brief Placeholder */}
            <div className="sidebar-section sidebar-section-muted">
              <h4 className="sidebar-section-title">Export</h4>
              <button className="btn-export" disabled title="Coming soon">
                📤 Copy LLM Brief (coming soon)
              </button>
            </div>
          </div>
        )}
      </aside>

      {/* Center - Graph or Editor */}
      <main className="architecture-center">
        {selectedSourceId ? (
          centerPane === 'graph' ? (
            <div className="graph-wrapper">
              <GraphView 
                sourceId={selectedSourceId}
                onNodeSelect={handleNodeSelect}
                focusNodeId={focusNodeId}
              />
            </div>
          ) : (
            <DesignEditor
              sourceId={selectedSourceId}
              notePath={selectedNote}
              onNoteChange={setSelectedNote}
              onFocusNode={handleFocusNode}
            />
          )
        ) : (
          <div className="center-empty">
            <p>Select a source to view its architecture</p>
          </div>
        )}
      </main>

      {/* Right Sidebar - Node Details */}
      <aside className={`architecture-sidebar architecture-sidebar-right ${rightSidebarCollapsed ? 'collapsed' : ''}`}>
        <button 
          className="sidebar-toggle sidebar-toggle-right"
          onClick={() => setRightSidebarCollapsed(!rightSidebarCollapsed)}
          title={rightSidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
        >
          {rightSidebarCollapsed ? '←' : '→'}
        </button>
        
        {!rightSidebarCollapsed && (
          <div className="sidebar-content">
            {selectedNode ? (
              <>
                <div className="sidebar-section">
                  <h4 className="sidebar-section-title">Selected File</h4>
                  <div className="node-detail">
                    <div className="node-detail-name">{selectedNode.label}</div>
                    <div className="node-detail-path">{selectedNode.id}</div>
                    <div className="node-detail-meta">
                      <span className={`language-badge language-${selectedNode.language}`}>
                        {selectedNode.language}
                      </span>
                      {selectedNode.folder && (
                        <span className="folder-badge">{selectedNode.folder}</span>
                      )}
                    </div>
                  </div>
                </div>

                <div className="sidebar-section">
                  <h4 className="sidebar-section-title">
                    Connections ({selectedNode.connections.length})
                  </h4>
                  {selectedNode.connections.length > 0 ? (
                    <ul className="connection-list">
                      {selectedNode.connections.map((conn, i) => (
                        <li key={i} className="connection-item">
                          <span className="connection-direction">
                            {conn.direction === 'out' ? '→' : '←'}
                          </span>
                          <span className="connection-id">{conn.id.split('/').pop()}</span>
                          <span className="connection-kind">({conn.kind})</span>
                        </li>
                      ))}
                    </ul>
                  ) : (
                    <p className="no-connections">No connections</p>
                  )}
                </div>

                <div className="sidebar-section">
                  <h4 className="sidebar-section-title">Actions</h4>
                  <button 
                    className="btn-action-full"
                    onClick={() => handleOpenNoteForNode(selectedNode.id)}
                  >
                    📝 Open Design Note
                  </button>
                </div>
              </>
            ) : (
              <div className="sidebar-section sidebar-placeholder">
                <p>Select a node in the graph to see details</p>
              </div>
            )}
          </div>
        )}
      </aside>
    </div>
  );
}
