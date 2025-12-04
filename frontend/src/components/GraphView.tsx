import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import ForceGraph2D from 'react-force-graph-2d';
import type { NodeObject, LinkObject } from 'react-force-graph-2d';
import {
  getGraph,
  getGraphStatus,
  rebuildGraph,
  type GraphNode,
  type GraphEdge,
  type GraphStatusResponse,
} from '../api';
import './GraphView.css';

interface GraphViewProps {
  sourceId: string;
}

// Extended node type for the force graph
interface GraphNodeObject extends NodeObject {
  id: string;
  label: string;
  language: string;
  folder: string;
  // Force graph adds these
  x?: number;
  y?: number;
  vx?: number;
  vy?: number;
}

// Extended link type for the force graph
interface GraphLinkObject extends LinkObject {
  source: string | GraphNodeObject;
  target: string | GraphNodeObject;
  kind: string;
}

// Color palette for languages
const LANGUAGE_COLORS: Record<string, string> = {
  rust: '#dea584',
  typescript: '#3178c6',
  javascript: '#f7df1e',
  default: '#6b7280',
};

export function GraphView({ sourceId }: GraphViewProps) {
  const [status, setStatus] = useState<GraphStatusResponse | null>(null);
  const [nodes, setNodes] = useState<GraphNodeObject[]>([]);
  const [links, setLinks] = useState<GraphLinkObject[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedNode, setSelectedNode] = useState<string | null>(null);
  const [hoveredNode, setHoveredNode] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [folderFilter, setFolderFilter] = useState('');

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const graphRef = useRef<any>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [dimensions, setDimensions] = useState({ width: 800, height: 600 });

  // Compute unique folders for filtering
  const folders = useMemo(() => {
    const folderSet = new Set<string>();
    nodes.forEach((node) => {
      if (node.folder) {
        // Add all folder levels
        const parts = node.folder.split('/');
        let current = '';
        parts.forEach((part) => {
          current = current ? `${current}/${part}` : part;
          folderSet.add(current);
        });
      }
    });
    return Array.from(folderSet).sort();
  }, [nodes]);

  // Compute neighbors for highlighting
  const highlightNodes = useMemo(() => {
    const set = new Set<string>();
    if (hoveredNode || selectedNode) {
      const focusNode = hoveredNode || selectedNode;
      set.add(focusNode!);
      links.forEach((link) => {
        const sourceId = typeof link.source === 'object' ? link.source.id : link.source;
        const targetId = typeof link.target === 'object' ? link.target.id : link.target;
        if (sourceId === focusNode) set.add(targetId);
        if (targetId === focusNode) set.add(sourceId);
      });
    }
    return set;
  }, [hoveredNode, selectedNode, links]);

  // Memoize graphData to prevent simulation restart on hover
  const graphData = useMemo(() => ({ nodes, links }), [nodes, links]);

  // Resize observer
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        setDimensions({
          width: entry.contentRect.width,
          height: entry.contentRect.height,
        });
      }
    });

    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  // Fetch graph status and data
  const fetchGraph = useCallback(async () => {
    setLoading(true);
    setError(null);

    try {
      const statusRes = await getGraphStatus(sourceId);
      setStatus(statusRes);

      if (statusRes.status === 'ready' || statusRes.status === 'stale') {
        const graphRes = await getGraph(sourceId, {
          folder: folderFilter || undefined,
        });

        // Transform to force graph format
        const graphNodes: GraphNodeObject[] = graphRes.nodes.map((n: GraphNode) => ({
          id: n.id,
          label: n.label,
          language: n.language,
          folder: n.folder,
        }));

        const graphLinks: GraphLinkObject[] = graphRes.edges.map((e: GraphEdge) => ({
          source: e.source,
          target: e.target,
          kind: e.kind,
        }));

        setNodes(graphNodes);
        setLinks(graphLinks);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load graph');
    } finally {
      setLoading(false);
    }
  }, [sourceId, folderFilter]);

  useEffect(() => {
    fetchGraph();
  }, [fetchGraph]);

  // Handle rebuild
  const handleRebuild = async () => {
    try {
      await rebuildGraph(sourceId);
      setStatus({ status: 'building' });
      // Poll for completion
      const poll = setInterval(async () => {
        const statusRes = await getGraphStatus(sourceId);
        setStatus(statusRes);
        if (statusRes.status === 'ready' || statusRes.status === 'error') {
          clearInterval(poll);
          fetchGraph();
        }
      }, 2000);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to rebuild graph');
    }
  };

  // Handle node click
  const handleNodeClick = useCallback((node: GraphNodeObject) => {
    setSelectedNode(node.id === selectedNode ? null : node.id);
  }, [selectedNode]);

  // Handle search
  const handleSearch = useCallback(() => {
    if (!searchQuery || !graphRef.current) return;

    const node = nodes.find(
      (n) =>
        n.id.toLowerCase().includes(searchQuery.toLowerCase()) ||
        n.label.toLowerCase().includes(searchQuery.toLowerCase())
    );

    if (node && node.x !== undefined && node.y !== undefined) {
      graphRef.current.centerAt(node.x, node.y, 1000);
      graphRef.current.zoom(2, 1000);
      setSelectedNode(node.id);
    }
  }, [searchQuery, nodes]);

  // Node color based on language
  const nodeColor = useCallback(
    (node: GraphNodeObject) => {
      const baseColor = LANGUAGE_COLORS[node.language] || LANGUAGE_COLORS.default;

      // Dim non-highlighted nodes when hovering/selecting
      if (highlightNodes.size > 0 && !highlightNodes.has(node.id)) {
        return '#d1d5db'; // Gray for non-highlighted
      }

      return baseColor;
    },
    [highlightNodes]
  );

  // Node label visibility based on zoom
  const nodeLabel = useCallback((node: GraphNodeObject) => {
    return node.label;
  }, []);

  // Link color
  const linkColor = useCallback(
    (link: GraphLinkObject) => {
      const sourceId = typeof link.source === 'object' ? link.source.id : link.source;
      const targetId = typeof link.target === 'object' ? link.target.id : link.target;

      // Dim non-highlighted links
      if (highlightNodes.size > 0) {
        const focusNode = hoveredNode || selectedNode;
        if (sourceId !== focusNode && targetId !== focusNode) {
          return 'rgba(200, 200, 200, 0.2)';
        }
      }

      return link.kind === 'import' ? '#94a3b8' : '#64748b';
    },
    [highlightNodes, hoveredNode, selectedNode]
  );

  if (loading) {
    return (
      <div className="graph-view graph-loading">
        <div className="loading-spinner"></div>
        <p>Loading graph...</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="graph-view graph-error">
        <p>Error: {error}</p>
        <button onClick={fetchGraph}>Retry</button>
      </div>
    );
  }

  if (status?.status === 'missing') {
    return (
      <div className="graph-view graph-empty">
        <p>No graph available yet.</p>
        <p className="hint">The graph will be built automatically after indexing completes.</p>
        <button onClick={handleRebuild}>Build Graph Now</button>
      </div>
    );
  }

  if (status?.status === 'building') {
    return (
      <div className="graph-view graph-building">
        <div className="loading-spinner"></div>
        <p>Building graph...</p>
        <p className="hint">This may take a moment for large projects.</p>
      </div>
    );
  }

  return (
    <div className="graph-view">
      <div className="graph-toolbar">
        <div className="graph-search">
          <input
            type="text"
            placeholder="Search files..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && handleSearch()}
          />
          <button onClick={handleSearch}>Search</button>
        </div>

        <div className="graph-filters">
          <select
            value={folderFilter}
            onChange={(e) => setFolderFilter(e.target.value)}
          >
            <option value="">All folders</option>
            {folders.map((folder) => (
              <option key={folder} value={folder}>
                {folder}
              </option>
            ))}
          </select>
        </div>

        <div className="graph-zoom">
          <button onClick={() => {
            const currentZoom = graphRef.current?.zoom();
            if (currentZoom) graphRef.current?.zoom(currentZoom * 1.5, 300);
          }} title="Zoom in">
            +
          </button>
          <button onClick={() => {
            const currentZoom = graphRef.current?.zoom();
            if (currentZoom) graphRef.current?.zoom(currentZoom / 1.5, 300);
          }} title="Zoom out">
            −
          </button>
          <button onClick={() => graphRef.current?.zoomToFit(400, 50)} title="Fit to view">
            Fit
          </button>
        </div>

        <div className="graph-actions">
          <button onClick={handleRebuild} title="Rebuild graph">
            Rebuild
          </button>
        </div>

        <div className="graph-stats">
          <span>{nodes.length} files</span>
          <span>{links.length} dependencies</span>
          {status?.built_at && (
            <span className="hint">
              Built: {new Date(status.built_at).toLocaleString()}
            </span>
          )}
        </div>
      </div>

      <div className="graph-container" ref={containerRef}>
        <ForceGraph2D
          ref={graphRef}
          width={dimensions.width}
          height={dimensions.height}
          graphData={graphData}
          nodeId="id"
          nodeLabel={nodeLabel}
          nodeColor={nodeColor}
          nodeRelSize={6}
          linkColor={linkColor}
          linkDirectionalArrowLength={3}
          linkDirectionalArrowRelPos={1}
          onNodeClick={handleNodeClick}
          onNodeHover={(node) => setHoveredNode(node ? (node as GraphNodeObject).id : null)}
          warmupTicks={100}
          cooldownTicks={0}
          enableNodeDrag={true}
          enableZoomInteraction={true}
          enablePanInteraction={true}
        />
      </div>

      {selectedNode && (
        <div className="graph-node-detail">
          <h4>Selected File</h4>
          <p className="node-id">{selectedNode}</p>
          <div className="node-connections">
            <h5>Connections</h5>
            <ul>
              {links
                .filter((l) => {
                  const sourceId = typeof l.source === 'object' ? l.source.id : l.source;
                  const targetId = typeof l.target === 'object' ? l.target.id : l.target;
                  return sourceId === selectedNode || targetId === selectedNode;
                })
                .map((l, i) => {
                  const sourceId = typeof l.source === 'object' ? l.source.id : l.source;
                  const targetId = typeof l.target === 'object' ? l.target.id : l.target;
                  const isOutgoing = sourceId === selectedNode;
                  return (
                    <li key={i}>
                      {isOutgoing ? '→ ' : '← '}
                      <span
                        className="connection-target"
                        onClick={() => {
                          setSelectedNode(isOutgoing ? targetId : sourceId);
                          const targetNode = nodes.find(
                            (n) => n.id === (isOutgoing ? targetId : sourceId)
                          );
                          if (targetNode && targetNode.x !== undefined && targetNode.y !== undefined && graphRef.current) {
                            graphRef.current.centerAt(targetNode.x, targetNode.y, 500);
                          }
                        }}
                      >
                        {isOutgoing ? targetId : sourceId}
                      </span>
                      <span className="connection-kind">({l.kind})</span>
                    </li>
                  );
                })}
            </ul>
          </div>
          <button onClick={() => setSelectedNode(null)}>Close</button>
        </div>
      )}
    </div>
  );
}
