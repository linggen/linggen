import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import ForceGraph2D from 'react-force-graph-2d';
import type { NodeObject, LinkObject } from 'react-force-graph-2d';
import {
  getGraphWithStatus,
  rebuildGraph,
  type GraphNode,
  type GraphEdge,
  type GraphStatusResponse,
} from '../api';

export interface SelectedNodeInfo {
  id: string;
  label: string;
  language: string;
  folder: string;
  connections: { id: string; kind: string; direction: 'in' | 'out' }[];
}

interface GraphViewProps {
  sourceId: string;
  onNodeSelect?: (node: SelectedNodeInfo | null) => void;
  focusNodeId?: string | null;
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

// Minimum zoom level (globalScale) at which to show file name labels.
// When zoomed out (globalScale < this), labels are hidden.
// Tweak this value if you want labels to appear earlier/later.
const LABEL_VISIBILITY_THRESHOLD = 1.2;

export function GraphView({ sourceId, onNodeSelect, focusNodeId }: GraphViewProps) {
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

  // Compute node degrees (number of connected links) for sizing
  const { nodeDegrees, maxDegree } = useMemo(() => {
    const degrees = new Map<string, number>();

    links.forEach((link) => {
      const sourceId = typeof link.source === 'object' ? link.source.id : link.source;
      const targetId = typeof link.target === 'object' ? link.target.id : link.target;

      if (!degrees.has(sourceId)) degrees.set(sourceId, 0);
      if (!degrees.has(targetId)) degrees.set(targetId, 0);

      degrees.set(sourceId, (degrees.get(sourceId) || 0) + 1);
      degrees.set(targetId, (degrees.get(targetId) || 0) + 1);
    });

    let max = 1;
    degrees.forEach((deg) => {
      if (deg > max) max = deg;
    });

    return { nodeDegrees: degrees, maxDegree: max };
  }, [links]);

  // Memoize graphData to prevent simulation restart on hover
  const graphData = useMemo(() => ({ nodes, links }), [nodes, links]);

  // Keep the graph canvas in sync with its container size.
  // Use both ResizeObserver (for flex/layout changes) and a window
  // resize fallback, since some environments can be finicky about firing resize events
  // through the observer alone.
  useEffect(() => {
    const updateSize = () => {
      const el = containerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();

      // Only update if dimensions are valid (not 0)
      if (rect.width > 0 && rect.height > 0) {
        setDimensions({
          width: rect.width,
          height: rect.height,
        });
      }
    };

    // Initial measurement - delayed to let DOM settle
    const initialTimeout = window.setTimeout(() => {
      updateSize();
    }, 10);

    // Also measure again after a longer delay for safety
    const fallbackTimeout = window.setTimeout(() => {
      updateSize();
    }, 100);

    // Observe direct size changes on the container
    const el = containerRef.current;
    let observer: ResizeObserver | null = null;
    if (el && 'ResizeObserver' in window) {
      observer = new ResizeObserver(() => updateSize());
      observer.observe(el);
    }

    // Fallback: also respond to window resize
    window.addEventListener('resize', updateSize);

    return () => {
      window.clearTimeout(initialTimeout);
      window.clearTimeout(fallbackTimeout);
      window.removeEventListener('resize', updateSize);
      if (observer && el) {
        observer.unobserve(el);
        observer.disconnect();
      }
    };
  }, []);

  // When the canvas size changes (e.g. window resized / fullscreen)
  // or when we first load nodes, auto-fit the graph so it uses the
  // available space instead of staying at the old zoom level.
  useEffect(() => {
    if (!graphRef.current) return;
    if (!nodes.length) return;

    // Small timeout lets ForceGraph2D apply the new width/height
    // before we ask it to zoomToFit.
    const id = window.setTimeout(() => {
      try {
        graphRef.current.zoomToFit(400, 50);
      } catch {
        // ignore if graphRef is not ready yet
      }
    }, 100);

    return () => window.clearTimeout(id);
  }, [dimensions.width, dimensions.height, nodes.length]);

  // Fetch graph status and data using optimized single-request API
  const fetchGraph = useCallback(async () => {
    setLoading(true);
    setError(null);

    // Remeasure container when starting to load (in case it wasn't sized correctly initially)
    window.setTimeout(() => {
      const el = containerRef.current;
      if (el) {
        const rect = el.getBoundingClientRect();
        if (rect.width > 0 && rect.height > 0) {
          setDimensions({
            width: rect.width,
            height: rect.height,
          });
        }
      }
    }, 10);

    try {
      // Use new optimized API: single request with optional focus/folder filters
      const graphWithStatus = await getGraphWithStatus(sourceId, {
        focus: focusNodeId || undefined,  // Focus on specific node if provided
        hops: focusNodeId ? 2 : undefined, // Include 2 hops when focused
        folder: folderFilter || undefined,
      });

      // Update status
      setStatus({
        status: graphWithStatus.status as 'missing' | 'stale' | 'ready' | 'building' | 'error',
        node_count: graphWithStatus.node_count,
        edge_count: graphWithStatus.edge_count,
        built_at: graphWithStatus.built_at || undefined,
      });

      // Only process graph if ready or stale
      if (graphWithStatus.status === 'ready' || graphWithStatus.status === 'stale') {
        // If focused/filtered query returned empty results, fetch full graph
        if (graphWithStatus.nodes.length === 0 && (focusNodeId || folderFilter)) {
          console.log('Focused/filtered graph returned 0 nodes, fetching full graph...');
          const fullGraph = await getGraphWithStatus(sourceId, {});

          // Transform full graph to force graph format
          const graphNodes: GraphNodeObject[] = fullGraph.nodes.map((n: GraphNode) => ({
            id: n.id,
            label: n.label,
            language: n.language,
            folder: n.folder,
          }));

          const graphLinks: GraphLinkObject[] = fullGraph.edges.map((e: GraphEdge) => ({
            source: e.source,
            target: e.target,
            kind: e.kind,
          }));

          setNodes(graphNodes);
          setLinks(graphLinks);
        } else {
          // Transform focused/filtered graph to force graph format
          const graphNodes: GraphNodeObject[] = graphWithStatus.nodes.map((n: GraphNode) => ({
            id: n.id,
            label: n.label,
            language: n.language,
            folder: n.folder,
          }));

          const graphLinks: GraphLinkObject[] = graphWithStatus.edges.map((e: GraphEdge) => ({
            source: e.source,
            target: e.target,
            kind: e.kind,
          }));

          setNodes(graphNodes);
          setLinks(graphLinks);
        }
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load graph');
    } finally {
      setLoading(false);

      // Remeasure one more time after loading completes
      window.setTimeout(() => {
        const el = containerRef.current;
        if (el) {
          const rect = el.getBoundingClientRect();
          if (rect.width > 0 && rect.height > 0) {
            setDimensions({
              width: rect.width,
              height: rect.height,
            });
          }
        }
      }, 50);
    }
  }, [sourceId, focusNodeId, folderFilter]);

  useEffect(() => {
    fetchGraph();
  }, [fetchGraph]);

  // Handle rebuild
  const handleRebuild = async () => {
    try {
      await rebuildGraph(sourceId);
      setStatus({ status: 'building' });
      // Poll for completion using optimized API
      const poll = setInterval(async () => {
        try {
          const graphWithStatus = await getGraphWithStatus(sourceId, {
            focus: focusNodeId || undefined,
            hops: focusNodeId ? 2 : undefined,
            folder: folderFilter || undefined,
          });

          setStatus({
            status: graphWithStatus.status as 'missing' | 'stale' | 'ready' | 'building' | 'error',
            node_count: graphWithStatus.node_count,
            edge_count: graphWithStatus.edge_count,
            built_at: graphWithStatus.built_at || undefined,
          });

          if (graphWithStatus.status === 'ready' || graphWithStatus.status === 'error') {
            clearInterval(poll);
            if (graphWithStatus.status === 'ready') {
              fetchGraph();
            }
          }
        } catch (error) {
          // Continue polling on errors
          console.error('Poll error:', error);
        }
      }, 2000);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to rebuild graph');
    }
  };

  // Build node info with connections for callback
  const buildNodeInfo = useCallback((nodeId: string): SelectedNodeInfo | null => {
    const node = nodes.find(n => n.id === nodeId);
    if (!node) return null;

    const connections: SelectedNodeInfo['connections'] = [];
    links.forEach((link) => {
      const sourceId = typeof link.source === 'object' ? link.source.id : link.source;
      const targetId = typeof link.target === 'object' ? link.target.id : link.target;

      if (sourceId === nodeId) {
        connections.push({ id: targetId, kind: link.kind, direction: 'out' });
      } else if (targetId === nodeId) {
        connections.push({ id: sourceId, kind: link.kind, direction: 'in' });
      }
    });

    return {
      id: node.id,
      label: node.label,
      language: node.language,
      folder: node.folder,
      connections,
    };
  }, [nodes, links]);

  // Handle node click
  const handleNodeClick = useCallback((node: GraphNodeObject) => {
    const newSelectedId = node.id === selectedNode ? null : node.id;
    setSelectedNode(newSelectedId);

    if (onNodeSelect) {
      if (newSelectedId) {
        const nodeInfo = buildNodeInfo(newSelectedId);
        onNodeSelect(nodeInfo);
      } else {
        onNodeSelect(null);
      }
    }
  }, [selectedNode, onNodeSelect, buildNodeInfo]);

  // Focus on node when focusNodeId changes
  useEffect(() => {
    if (focusNodeId && graphRef.current && nodes.length > 0) {
      const node = nodes.find(n => n.id === focusNodeId);
      if (node && node.x !== undefined && node.y !== undefined) {
        graphRef.current.centerAt(node.x, node.y, 1000);
        graphRef.current.zoom(2, 1000);
        setSelectedNode(focusNodeId);

        if (onNodeSelect) {
          const nodeInfo = buildNodeInfo(focusNodeId);
          onNodeSelect(nodeInfo);
        }
      }
    }
  }, [focusNodeId, nodes, buildNodeInfo, onNodeSelect]);

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

  // Custom node rendering: circle sized by degree + file name label
  const nodeCanvasObject = useCallback(
    (node: NodeObject, ctx: CanvasRenderingContext2D, globalScale: number) => {
      const n = node as GraphNodeObject;
      const x = n.x ?? 0;
      const y = n.y ?? 0;

      // Base radius scaled by node degree
      const degree = nodeDegrees.get(n.id) ?? 1;
      const minRadius = 4;
      const maxExtraRadius = 10;
      const radius =
        minRadius +
        (maxDegree > 1 ? (degree / maxDegree) * maxExtraRadius : 0);

      // Draw node circle
      ctx.beginPath();
      ctx.arc(x, y, radius, 0, 2 * Math.PI, false);
      ctx.fillStyle = nodeColor(n);
      ctx.fill();

      // Emphasize highlighted nodes with a stroke
      if (highlightNodes.size === 0 || highlightNodes.has(n.id)) {
        ctx.lineWidth = 1.5;
        const rootTheme = document.documentElement.getAttribute('data-theme');
        const isLight = rootTheme === 'light' || (!rootTheme && window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches);
        ctx.strokeStyle = isLight ? 'rgba(0,0,0,0.1)' : '#111827';
        ctx.stroke();
      }

      // Draw file name label below the node
      const label = n.label;
      // Only show labels when zoomed in enough, Obsidian-style.
      if (label && globalScale >= LABEL_VISIBILITY_THRESHOLD) {
        const rootTheme = document.documentElement.getAttribute('data-theme');
        const isLight = rootTheme === 'light' || (!rootTheme && window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches);
        const fontSize = Math.max(10, 14 / globalScale);
        ctx.font = `${fontSize}px system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'top';

        const textY = y + radius + 2;

        // Text outline for contrast
        ctx.lineWidth = 3;
        ctx.strokeStyle = isLight ? 'rgba(255, 255, 255, 0.85)' : 'rgba(15, 23, 42, 0.85)';
        ctx.strokeText(label, x, textY);

        // Main text color
        ctx.fillStyle = isLight ? '#1a1a1a' : '#f9fafb';
        ctx.fillText(label, x, textY);
      }
    },
    [highlightNodes, maxDegree, nodeColor, nodeDegrees]
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

      // Style edge types:
      // - import: normal file import
      // - workspace_crate: cross-crate edge (workspace member)
      if (link.kind === 'workspace_crate') return '#a78bfa'; // purple
      if (link.kind === 'import') return '#94a3b8'; // gray-blue
      return '#64748b';
    },
    [highlightNodes, hoveredNode, selectedNode]
  );

  if (loading) {
    return (
      <div className="flex flex-col flex-1 items-center justify-center h-full bg-[var(--bg-app)] text-[var(--text-secondary)] p-8">
        <div className="w-10 h-10 border-3 border-slate-700 border-t-blue-500 rounded-full animate-spin mb-4"></div>
        <p>Loading graph...</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex flex-col flex-1 items-center justify-center h-full bg-[var(--bg-app)] text-red-400 p-8">
        <p>Error: {error}</p>
        <button onClick={fetchGraph} className="btn-primary mt-4">Retry</button>
      </div>
    );
  }

  if (status?.status === 'missing') {
    return (
      <div className="flex flex-col flex-1 items-center justify-center h-full bg-[var(--bg-app)] text-[var(--text-secondary)] p-8">
        <p>No graph available yet.</p>
        <p className="text-xs text-[var(--text-muted)] mt-2 italic">The graph will be built automatically after indexing completes.</p>
        <button onClick={handleRebuild} className="btn-primary mt-4">Build Graph Now</button>
      </div>
    );
  }

  if (status?.status === 'building') {
    return (
      <div className="flex flex-col flex-1 items-center justify-center h-full bg-[var(--bg-app)] text-[var(--text-secondary)] p-8">
        <div className="w-10 h-10 border-3 border-slate-700 border-t-blue-500 rounded-full animate-spin mb-4"></div>
        <p>Building graph...</p>
        <p className="text-xs text-[var(--text-muted)] mt-2 italic">This may take a moment for large projects.</p>
      </div>
    );
  }

  return (
    <div className="flex flex-col flex-1 h-full bg-[var(--bg-app)] rounded-lg overflow-hidden relative">
      <div className="flex items-center gap-4 px-4 py-3 bg-black/5 border-b border-[var(--border-color)] flex-wrap">
        <div className="flex gap-2">
          <input
            type="text"
            placeholder="Search files..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && handleSearch()}
            className="px-3 py-1.5 bg-[var(--bg-app)] border border-[var(--border-color)] rounded text-sm text-[var(--text-primary)] placeholder:text-[var(--text-secondary)] outline-none focus:border-[var(--accent)] min-w-[200px]"
          />
          <button onClick={handleSearch} className="btn-secondary !normal-case">Search</button>
        </div>

        <div className="flex items-center gap-2">
          <select
            value={folderFilter}
            onChange={(e) => setFolderFilter(e.target.value)}
            className="px-3 py-1.5 bg-[var(--bg-app)] border border-[var(--border-color)] rounded text-sm text-[var(--text-primary)] outline-none cursor-pointer focus:border-[var(--accent)]"
          >
            <option value="">All folders</option>
            {folders.map((folder) => (
              <option key={folder} value={folder}>
                {folder}
              </option>
            ))}
          </select>
        </div>

        <div className="flex gap-1">
          <button onClick={() => {
            const currentZoom = graphRef.current?.zoom();
            if (currentZoom) graphRef.current?.zoom(currentZoom * 1.5, 300);
          }} title="Zoom in" className="btn-secondary !px-2.5 font-bold">
            +
          </button>
          <button onClick={() => {
            const currentZoom = graphRef.current?.zoom();
            if (currentZoom) graphRef.current?.zoom(currentZoom / 1.5, 300);
          }} title="Zoom out" className="btn-secondary !px-2.5 font-bold">
            −
          </button>
          <button onClick={() => graphRef.current?.zoomToFit(400, 50)} title="Fit to view" className="btn-secondary !px-2.5">
            Fit
          </button>
        </div>

        <div className="ml-auto">
          <button onClick={handleRebuild} title="Rebuild graph" className="btn-secondary">
            Rebuild
          </button>
        </div>

        <div className="flex items-center gap-4 text-[11px] text-[var(--text-secondary)] tracking-tight">
          <span className="whitespace-nowrap">{nodes.length} files</span>
          <span className="whitespace-nowrap">{links.length} dependencies</span>
          {status?.built_at && (
            <span className="opacity-60 italic hidden sm:inline">
              Built: {new Date(status.built_at).toLocaleTimeString()}
            </span>
          )}
        </div>
      </div>

      <div className="flex-1 min-h-0 relative" ref={containerRef}>
        <ForceGraph2D
          ref={graphRef}
          width={dimensions.width}
          height={dimensions.height}
          graphData={graphData}
          nodeId="id"
          nodeLabel={nodeLabel}
          nodeColor={nodeColor}
          nodeRelSize={6}
          nodeCanvasObject={nodeCanvasObject}
          nodeCanvasObjectMode={() => 'replace'}
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
        <div className="absolute bottom-4 right-4 w-[300px] max-h-[300px] bg-[var(--bg-sidebar)]/80 backdrop-blur-md border border-[var(--border-color)] rounded-lg p-4 overflow-y-auto z-10 shadow-2xl flex flex-col gap-3">
          <div>
            <h4 className="m-0 text-xs font-semibold text-[var(--text-secondary)] uppercase tracking-wider mb-1">Selected File</h4>
            <p className="m-0 font-mono text-[11px] text-blue-400 break-all">{selectedNode}</p>
          </div>

          <div className="flex flex-col gap-2">
            <h5 className="m-0 text-[10px] font-bold text-[var(--text-muted)] uppercase tracking-widest">Connections</h5>
            <ul className="list-none p-0 m-0 max-h-[150px] overflow-y-auto flex flex-col gap-1">
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
                  const otherNodeId = isOutgoing ? targetId : sourceId;
                  return (
                    <li key={i} className="text-[11px] text-[var(--text-secondary)] flex items-center gap-1.5">
                      <span className="opacity-50">{isOutgoing ? '→' : '←'}</span>
                      <button
                        className="text-left text-blue-400 hover:underline break-all bg-transparent border-none p-0 cursor-pointer flex-1"
                        onClick={() => {
                          setSelectedNode(otherNodeId);
                          const targetNode = nodes.find(
                            (n) => n.id === otherNodeId
                          );
                          if (targetNode && targetNode.x !== undefined && targetNode.y !== undefined && graphRef.current) {
                            graphRef.current.centerAt(targetNode.x, targetNode.y, 500);
                          }
                        }}
                      >
                        {otherNodeId}
                      </button>
                      <span className="text-[9px] text-[var(--text-muted)] opacity-70">({l.kind})</span>
                    </li>
                  );
                })}
            </ul>
          </div>
          <button onClick={() => setSelectedNode(null)} className="btn-secondary !w-full !py-1 text-[10px]">Close</button>
        </div>
      )}
    </div>
  );
}
