import { useEffect, useMemo, useState } from 'react';
import './App.css';
import './index.css';

import { API_BASE } from './api';

type Tab = 'explain' | 'graph' | 'prompts';

function getParam(name: string): string | null {
  return new URLSearchParams(window.location.search).get(name);
}

function buildQuery({ selection, symbol, filePath }: { selection?: string; symbol?: string; filePath?: string }) {
  const parts: string[] = [];
  if (selection) parts.push(`Selection:\n${selection}`);
  if (symbol) parts.push(`Symbol: ${symbol}`);
  if (filePath) parts.push(`File: ${filePath}`);
  return parts.join('\n\n').trim();
}

export default function ExtensionApp() {
  const [tab, setTab] = useState<Tab>('explain');
  const [queryResult, setQueryResult] = useState<string>('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Graph
  const [graphFocus, setGraphFocus] = useState<string>('');
  const [graphText, setGraphText] = useState<string>('');

  // Prompts
  const [promptList, setPromptList] = useState<Array<{ path: string; name: string }>>([]);
  const [selectedPromptPath, setSelectedPromptPath] = useState<string>('');
  const [promptContent, setPromptContent] = useState<string>('');
  const [promptSaving, setPromptSaving] = useState(false);

  const sourceId = getParam('source_id') || '';
  const filePath = getParam('file_path') || '';
  const selection = getParam('selection') || '';
  const symbol = getParam('symbol') || '';
  const initialTab = (getParam('tab') as Tab | null) || 'explain';

  const builtQuery = useMemo(
    () => buildQuery({ selection: selection || undefined, symbol: symbol || undefined, filePath: filePath || undefined }),
    [selection, symbol, filePath],
  );

  useEffect(() => {
    setTab(initialTab);
  }, [initialTab]);

  const runExplainAcrossProjects = async () => {
    if (!sourceId) {
      setError('Missing source_id in URL. Provide ?source_id=...');
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const body = {
        query: builtQuery || 'Explain across projects',
        limit: 3,
        exclude_source_id: sourceId,
      };
      const resp = await fetch(`${API_BASE}/api/query`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (!resp.ok) throw new Error(`Query failed: ${resp.status} ${resp.statusText}`);
      const data = await resp.json();
      // backend/api/src/handlers/search.rs returns { results: Chunk[] }
      const chunks = (data?.results || []) as Array<{ source_id: string; document_id: string; content: string }>;
      const text = chunks
        .slice(0, 3)
        .map((c, i) => `--- Chunk ${i + 1} [${c.source_id}] ---\nFile: ${c.document_id}\n\n${c.content}\n`)
        .join('\n');
      setQueryResult(text || 'No results.');
    } catch (e: any) {
      setError(String(e?.message || e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (tab === 'explain') {
      runExplainAcrossProjects();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab, sourceId, builtQuery]);

  const loadGraph = async (focusPath: string) => {
    if (!sourceId) {
      setError('Missing source_id in URL. Provide ?source_id=...');
      return;
    }
    const fp = focusPath || filePath;
    if (!fp) {
      setError('Missing file_path to focus graph. Provide ?file_path=...');
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const url = new URL(`${API_BASE}/api/sources/${sourceId}/graph/focus`);
      url.searchParams.set('file_path', fp);
      url.searchParams.set('hops', '1');
      const resp = await fetch(url.toString());
      if (!resp.ok) throw new Error(`Graph failed: ${resp.status} ${resp.statusText}`);
      const data = await resp.json();
      const nodes = (data?.nodes || []) as Array<{ id: string; label: string }>;
      const edges = (data?.edges || []) as Array<{ source: string; target: string; kind: string }>;
      const out =
        `Focus: ${fp}\n\n` +
        `Nodes (${nodes.length}):\n` +
        nodes.map((n) => `- ${n.id} (${n.label})`).join('\n') +
        `\n\nEdges (${edges.length}):\n` +
        edges.map((e) => `- ${e.source} -> ${e.target} (${e.kind})`).join('\n');
      setGraphText(out);
      setGraphFocus(fp);
    } catch (e: any) {
      setError(String(e?.message || e));
    } finally {
      setLoading(false);
    }
  };

  const loadPrompts = async () => {
    if (!sourceId) return;
    const resp = await fetch(`${API_BASE}/api/sources/${sourceId}/prompts`);
    if (!resp.ok) return;
    const data = await resp.json();
    const prompts = (data?.prompts || []) as Array<{ path: string; name: string }>;
    setPromptList(prompts);
  };

  const openPrompt = async (path: string) => {
    if (!sourceId || !path) return;
    setSelectedPromptPath(path);
    setError(null);
    try {
      const resp = await fetch(`${API_BASE}/api/sources/${sourceId}/prompts/${encodeURIComponent(path)}`);
      if (!resp.ok) throw new Error(`Failed to load prompt: ${resp.statusText}`);
      const data = await resp.json();
      setPromptContent(data?.content || '');
    } catch (e: any) {
      setError(String(e?.message || e));
    }
  };

  const savePrompt = async () => {
    if (!sourceId || !selectedPromptPath) return;
    setPromptSaving(true);
    setError(null);
    try {
      const resp = await fetch(`${API_BASE}/api/sources/${sourceId}/prompts/${encodeURIComponent(selectedPromptPath)}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ content: promptContent }),
      });
      if (!resp.ok) throw new Error(`Failed to save prompt: ${resp.statusText}`);
      await loadPrompts();
    } catch (e: any) {
      setError(String(e?.message || e));
    } finally {
      setPromptSaving(false);
    }
  };

  useEffect(() => {
    if (tab === 'graph') {
      loadGraph(graphFocus || filePath);
    } else if (tab === 'prompts') {
      loadPrompts();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab, sourceId]);

  const copyToClipboard = async () => {
    try {
      await navigator.clipboard.writeText(queryResult);
    } catch {
      // ignore
    }
  };

  return (
    <div
      style={{
        height: '100vh',
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--bg-app)',
        color: 'var(--text-primary)',
      }}
    >
      <div
        style={{
          padding: '10px 14px',
          borderBottom: '1px solid var(--border-color)',
          display: 'flex',
          gap: 8,
          alignItems: 'center',
          background: 'var(--bg-surface)',
        }}
      >
        <div style={{ fontWeight: 600, fontSize: 14 }}>Linggen Panel</div>
        <div style={{ display: 'flex', gap: 6, marginLeft: 10 }}>
          <button className={`btn-small ${tab === 'explain' ? 'btn-primary' : ''}`} onClick={() => setTab('explain')}>
            Explain
          </button>
          <button className={`btn-small ${tab === 'graph' ? 'btn-primary' : ''}`} onClick={() => setTab('graph')}>
            Graph
          </button>
          <button className={`btn-small ${tab === 'prompts' ? 'btn-primary' : ''}`} onClick={() => setTab('prompts')}>
            Prompts
          </button>
        </div>
        <div
          style={{
            marginLeft: 'auto',
            fontFamily: 'monospace',
            fontSize: 12,
            color: 'var(--text-muted)',
            background: 'var(--bg-muted)',
            padding: '4px 8px',
            borderRadius: 6,
            border: '1px solid var(--border-color)',
          }}
          title="Current file"
        >
          {filePath || '(no file)'}
        </div>
      </div>

      <div style={{ flex: 1, minHeight: 0, padding: 16, overflow: 'auto' }}>
        {!sourceId && (
          <div
            style={{
              padding: '12px 14px',
              border: '1px solid var(--border-color)',
              borderRadius: 8,
              background: 'var(--bg-surface)',
              marginBottom: 12,
              color: 'var(--text-muted)',
            }}
          >
            Missing <code>source_id</code> in URL. Provide <code>?source_id=...</code> (plus <code>file_path</code>,
            optional <code>selection</code>/<code>symbol</code>).
          </div>
        )}

        {tab === 'explain' && (
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 10,
              background: 'var(--bg-surface)',
              border: '1px solid var(--border-color)',
              borderRadius: 10,
              padding: 14,
              boxShadow: '0 4px 12px rgba(0,0,0,0.12)',
            }}
          >
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <button className="btn-small" onClick={runExplainAcrossProjects} disabled={loading || !sourceId}>
                {loading ? 'Running…' : 'Re-run'}
              </button>
              <button className="btn-small" onClick={copyToClipboard} disabled={!queryResult}>
                Copy
              </button>
              <div style={{ marginLeft: 'auto', fontSize: 12, color: 'var(--text-muted)' }}>
                Using source_id: <code>{sourceId || '—'}</code>
              </div>
            </div>
            {error && (
              <div
                style={{
                  color: 'var(--error)',
                  background: 'rgba(255,0,0,0.08)',
                  border: '1px solid rgba(255,0,0,0.25)',
                  padding: '8px 10px',
                  borderRadius: 6,
                }}
              >
                {error}
              </div>
            )}
            <pre
              style={{
                whiteSpace: 'pre-wrap',
                fontSize: 12,
                lineHeight: 1.5,
                background: 'var(--bg-muted)',
                border: '1px solid var(--border-color)',
                borderRadius: 8,
                padding: 12,
                minHeight: 260,
              }}
            >
              {queryResult || 'Results will appear here.'}
            </pre>
          </div>
        )}

        {tab === 'graph' && (
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 10,
              background: 'var(--bg-surface)',
              border: '1px solid var(--border-color)',
              borderRadius: 10,
              padding: 14,
              boxShadow: '0 4px 12px rgba(0,0,0,0.12)',
            }}
          >
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <button className="btn-small" onClick={() => loadGraph(graphFocus || filePath)} disabled={loading || !sourceId}>
                {loading ? 'Loading…' : 'Reload'}
              </button>
              <button className="btn-small" onClick={() => setGraphFocus(filePath)} disabled={!filePath}>
                Focus current file
              </button>
              <input
                value={graphFocus || filePath || ''}
                onChange={(e) => setGraphFocus(e.target.value)}
                placeholder="Focus file path..."
                style={{
                  flex: 1,
                  minWidth: 0,
                  padding: '6px 8px',
                  borderRadius: 6,
                  border: '1px solid var(--border-color)',
                  background: 'var(--bg-content)',
                  color: 'var(--text-primary)',
                  fontSize: 12,
                }}
              />
            </div>
            {error && (
              <div
                style={{
                  color: 'var(--error)',
                  background: 'rgba(255,0,0,0.08)',
                  border: '1px solid rgba(255,0,0,0.25)',
                  padding: '8px 10px',
                  borderRadius: 6,
                }}
              >
                {error}
              </div>
            )}
            <pre
              style={{
                whiteSpace: 'pre-wrap',
                fontSize: 12,
                lineHeight: 1.45,
                background: 'var(--bg-muted)',
                border: '1px solid var(--border-color)',
                borderRadius: 8,
                padding: 12,
                minHeight: 220,
              }}
            >
              {graphText || 'Focused graph neighborhood will appear here.'}
            </pre>
          </div>
        )}

        {tab === 'prompts' && (
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 10,
              background: 'var(--bg-surface)',
              border: '1px solid var(--border-color)',
              borderRadius: 10,
              padding: 14,
              boxShadow: '0 4px 12px rgba(0,0,0,0.12)',
            }}
          >
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <button className="btn-small" onClick={loadPrompts} disabled={loading || !sourceId}>
                Refresh
              </button>
              <select
                value={selectedPromptPath}
                onChange={(e) => openPrompt(e.target.value)}
                style={{
                  flex: 1,
                  maxWidth: 420,
                  padding: '6px 8px',
                  borderRadius: 6,
                  border: '1px solid var(--border-color)',
                  background: 'var(--bg-content)',
                  color: 'var(--text-primary)',
                  fontSize: 12,
                }}
              >
                <option value="">Select a prompt…</option>
                {promptList.map((p) => (
                  <option key={p.path} value={p.path}>
                    {p.name}
                  </option>
                ))}
              </select>
              <button className="btn-small btn-primary" onClick={savePrompt} disabled={!selectedPromptPath || promptSaving}>
                {promptSaving ? 'Saving…' : 'Save'}
              </button>
            </div>
            {error && (
              <div
                style={{
                  color: 'var(--error)',
                  background: 'rgba(255,0,0,0.08)',
                  border: '1px solid rgba(255,0,0,0.25)',
                  padding: '8px 10px',
                  borderRadius: 6,
                }}
              >
                {error}
              </div>
            )}
            <textarea
              value={promptContent}
              onChange={(e) => setPromptContent(e.target.value)}
              placeholder="Select a prompt to edit..."
              style={{
                width: '100%',
                minHeight: 420,
                background: 'var(--bg-content)',
                color: 'var(--text-primary)',
                border: '1px solid var(--border-color)',
                borderRadius: 8,
                padding: 12,
                fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
                fontSize: 12,
                lineHeight: 1.5,
              }}
            />
          </div>
        )}
      </div>
    </div>
  );
}


