import React, { useState, useEffect } from 'react'
import { enhancePrompt, listResources, getAppSettings, type EnhancedPromptResponse, type Resource, type IntentType, type AppSettings } from '../api'
import { Chat } from '../components/Chat'

type ComposerBlockType = 'chunk' | 'question' | 'path'

interface ComposerBlock {
    id: string
    type: ComposerBlockType
    sourceId?: string
    label: string
    text: string
}

interface AvailableChunk {
    id: string
    index: number
    sourceId: string
    filePath: string
    preview: string
    fullText: string
}

export function AssistantView() {
    const [query, setQuery] = useState('')

    // Enhancement result + pipeline state
    const [processing, setProcessing] = useState(false)
    const [result, setResult] = useState<EnhancedPromptResponse | null>(null)
    const [error, setError] = useState('')
    const [copied, setCopied] = useState(false)

    const [availableChunks, setAvailableChunks] = useState<AvailableChunk[]>([])
    const [selectedChunkIds, setSelectedChunkIds] = useState<Set<string>>(new Set())
    const [composerBlocks, setComposerBlocks] = useState<ComposerBlock[]>([])
    const [fullTextModal, setFullTextModal] = useState<{ open: boolean; chunk?: AvailableChunk }>({
        open: false,
    })
    const [chunkLimit, setChunkLimit] = useState<number>(3)

    // Sources & profiles state
    // Sources state
    const [sources, setSources] = useState<Resource[]>([])

    // App settings (for LLM enabled status)
    const [appSettings, setAppSettings] = useState<AppSettings | null>(null)

    useEffect(() => {
        const loadSources = async () => {
            try {
                const res = await listResources()
                setSources(res.resources)
            } catch (err) {
                console.error('Failed to load sources:', err)
            }
        }

        const loadSettings = async () => {
            try {
                const settings = await getAppSettings()
                setAppSettings(settings)
            } catch (err) {
                console.error('Failed to load app settings:', err)
            }
        }

        loadSources()
        loadSettings()
    }, [])


    const handleEnhance = async (e: React.FormEvent) => {
        e.preventDefault()
        if (!query.trim()) return

        setProcessing(true)
        setError('')
        setResult(null)
        setCopied(false)
        setAvailableChunks([])
        setSelectedChunkIds(new Set())
        setComposerBlocks([])

        try {
            // Direct call to enhancePrompt which handles intent + enhancement (always full_code)
            const enhanced = await enhancePrompt(query, 'full_code')
            setResult(enhanced)

            // Build available chunks list from result
            const meta = enhanced.context_metadata ?? []
            const chunks: AvailableChunk[] = enhanced.context_chunks.map((content, index) => {
                const m = meta[index]
                // Prefer file_path from metadata, then document_id; only fall back to generic label if missing
                const filePath = m?.file_path || m?.document_id || `Chunk ${index + 1}`

                const preview =
                    content.length > 400 ? content.slice(0, 400) + '…' : content
                return {
                    id: `${index}`,
                    index,
                    sourceId: m?.source_id ?? '',
                    filePath,
                    preview,
                    fullText: content,
                }
            })
            setAvailableChunks(chunks)

            // Auto-select top K chunks (up to 3) and seed composer with paths only
            const autoSelectCount = Math.min(chunkLimit, chunks.length)
            const initialSelected = new Set<string>()
            const initialBlocks: ComposerBlock[] = []

            // Question block at the bottom
            const questionBlock: ComposerBlock = {
                id: 'question',
                type: 'question',
                label: 'Your Question',
                text: enhanced.original_query,
            }

            for (let i = 0; i < autoSelectCount; i++) {
                const c = chunks[i]
                const pathId = `path-${c.id}`
                initialSelected.add(pathId)
                initialBlocks.push({
                    id: pathId,
                    type: 'path',
                    sourceId: c.sourceId,
                    label: c.filePath || `Chunk ${c.index + 1}`,
                    text: `File: ${c.filePath}`,
                })
            }

            initialBlocks.push(questionBlock)

            setSelectedChunkIds(initialSelected)
            setComposerBlocks(initialBlocks)
        } catch (error) {
            setError(`${error}`)
        } finally {
            setProcessing(false)
        }
    }

    const composedPrompt =
        composerBlocks.length > 0
            ? composerBlocks
                .map((b) => {
                    if (b.type === 'chunk') {
                        return `--- Context from: ${b.label} ---\n\n${b.text}`
                    }
                    return b.text
                })
                .join('\n\n')
            : result?.enhanced_prompt ?? ''

    const handleCopy = async () => {
        if (!composedPrompt) return
        try {
            await navigator.clipboard.writeText(composedPrompt)
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
        <div className="flex flex-1 flex-col overflow-hidden bg-[var(--bg-content)] text-[var(--text-primary)]">
            <div className="flex flex-1 overflow-hidden">
                {/* Left + middle columns */}
                <div className="flex-1 flex flex-col overflow-y-auto p-6 gap-6">
                    {/* Query Input */}
                    <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-lg p-5">
                        <form onSubmit={handleEnhance} className="flex flex-col gap-4">
                            <div className="flex flex-col gap-2">
                                <label htmlFor="query" className="text-[11px] font-semibold text-[var(--text-secondary)] uppercase tracking-wider">Your Query</label>
                                <textarea
                                    id="query"
                                    value={query}
                                    onChange={(e) => setQuery(e.target.value)}
                                    placeholder="e.g., 'Fix the timeout bug in auth service' or 'Explain how the login function works'"
                                    rows={3}
                                    required
                                    className="w-full rounded-md border border-[var(--border-color)] bg-[var(--bg-app)] p-3 text-sm text-[var(--text-primary)] placeholder:text-[var(--text-secondary)] outline-none focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)]/30 transition-all"
                                />
                            </div>
                            <button type="submit" disabled={processing} className="btn-primary self-start px-6 py-2">
                                {processing ? '✨ Enhancing...' : '✨ Enhance Prompt'}
                            </button>
                        </form>
                    </section>

                    {error && <div className="p-3 bg-red-500/10 border border-red-500/20 rounded text-red-400 text-sm font-medium">{error}</div>}

                    {/* Two-column composer layout */}
                    {result && (
                        <section className="grid grid-cols-1 lg:grid-cols-2 gap-6 items-start">
                            {/* Left: Sources + Context Chunks */}
                            <div className="flex flex-col gap-6">
                                <div className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-lg p-5">
                                    <h4 className="text-sm font-semibold text-[var(--text-active)] mb-3 pb-2 border-b border-[var(--border-color)]">Sources</h4>
                                    <div className="flex flex-wrap gap-x-4 gap-y-1 mb-4">
                                        <div className="text-[11px] text-[var(--text-secondary)] font-medium uppercase tracking-wide">
                                            Intent: <span className="text-[var(--text-primary)] normal-case">{formatIntent(result.intent)}</span>
                                        </div>
                                        <div className="text-[11px] text-[var(--text-secondary)] font-medium uppercase tracking-wide">
                                            Retrieved chunks: <span className="text-[var(--text-primary)]">{availableChunks.length}</span>
                                        </div>
                                    </div>
                                    <div className="flex flex-col gap-2.5">
                                        {sources.map((src) => (
                                            <div key={src.id} className="bg-[var(--bg-app)] border border-[var(--border-color)] rounded p-3 hover:border-[var(--accent)]/50 transition-colors">
                                                <div className="flex items-center justify-between mb-1">
                                                    <span className="text-xs font-semibold text-[var(--text-active)] truncate mr-2">{src.name}</span>
                                                    <span className="text-[9px] px-1.5 py-0.5 rounded bg-[var(--accent)]/10 text-[var(--accent)] font-bold border border-[var(--accent)]/20 uppercase tracking-tight">
                                                        {src.resource_type}
                                                    </span>
                                                </div>
                                                <div className="text-[10px] text-[var(--text-secondary)] font-mono truncate">{src.path}</div>
                                            </div>
                                        ))}
                                        {sources.length === 0 && (
                                            <p className="text-xs text-[var(--text-secondary)] italic">
                                                No sources yet.
                                            </p>
                                        )}
                                    </div>
                                </div>

                                {/* Context Chunks */}
                                <div className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-lg p-5 flex flex-col gap-4">
                                    <div className="flex items-center justify-between pb-2 border-b border-[var(--border-color)]">
                                        <h4 className="text-sm font-semibold text-[var(--text-active)]">Context Chunks</h4>
                                        <div className="flex items-center gap-2">
                                            <span className="text-[11px] text-[var(--text-secondary)] font-semibold uppercase tracking-wider">Top K:</span>
                                            <select
                                                value={chunkLimit}
                                                onChange={(e) => setChunkLimit(parseInt(e.target.value, 10))}
                                                className="bg-[var(--bg-app)] border border-[var(--border-color)] rounded text-[11px] px-1.5 py-0.5 text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
                                            >
                                                <option value={3}>3</option>
                                                <option value={5}>5</option>
                                                <option value={8}>8</option>
                                                <option value={10}>10</option>
                                            </select>
                                        </div>
                                    </div>
                                    {availableChunks.length === 0 && (
                                        <div className="p-8 text-center text-xs text-[var(--text-secondary)] italic">
                                            No context chunks retrieved yet. Try a different query or
                                            strategy.
                                        </div>
                                    )}
                                    <div className="flex flex-col gap-3">
                                        {availableChunks.slice(0, chunkLimit).map((chunk) => {
                                            const pathId = `path-${chunk.id}`
                                            const fullId = `full-${chunk.id}`
                                            const hasPath = selectedChunkIds.has(pathId)
                                            const hasFull = selectedChunkIds.has(fullId)
                                            const isSelected = hasPath || hasFull

                                            return (
                                                <div
                                                    key={chunk.id}
                                                    className={`bg-[var(--bg-app)] border rounded-lg p-3 transition-all ${isSelected ? 'border-[var(--accent)] bg-[var(--accent)]/5' : 'border-[var(--border-color)]'}`}
                                                >
                                                    <div className="mb-2">
                                                        <button
                                                            onClick={(e) => {
                                                                e.preventDefault()
                                                                setFullTextModal({ open: true, chunk })
                                                            }}
                                                            className="text-xs font-medium text-[var(--accent)] hover:underline truncate block w-full text-left"
                                                        >
                                                            {chunk.filePath}
                                                        </button>
                                                    </div>

                                                    <div className="flex items-center gap-4">
                                                        <label className="flex items-center gap-2 cursor-pointer group">
                                                            <input
                                                                type="checkbox"
                                                                checked={hasPath}
                                                                onChange={(e) => {
                                                                    e.stopPropagation()
                                                                    const next = new Set(selectedChunkIds)
                                                                    if (hasPath) {
                                                                        next.delete(pathId)
                                                                        setSelectedChunkIds(next)
                                                                        setComposerBlocks((prev) =>
                                                                            prev.filter((b) => b.id !== pathId),
                                                                        )
                                                                    } else {
                                                                        next.add(pathId)
                                                                        setSelectedChunkIds(next)
                                                                        setComposerBlocks((prev) => [
                                                                            {
                                                                                id: pathId,
                                                                                type: 'path',
                                                                                sourceId: chunk.sourceId,
                                                                                label: chunk.filePath,
                                                                                text: `File: ${chunk.filePath}`,
                                                                            },
                                                                            ...prev,
                                                                        ])
                                                                    }
                                                                }}
                                                                className="w-3.5 h-3.5 rounded border-[var(--border-color)] text-[var(--accent)] focus:ring-[var(--accent)] bg-[var(--bg-app)]"
                                                            />
                                                            <span className="text-[11px] text-[var(--text-secondary)] group-hover:text-[var(--text-primary)] transition-colors">Add path</span>
                                                        </label>

                                                        <label className="flex items-center gap-2 cursor-pointer group">
                                                            <input
                                                                type="checkbox"
                                                                checked={hasFull}
                                                                onChange={(e) => {
                                                                    e.stopPropagation()
                                                                    const next = new Set(selectedChunkIds)
                                                                    if (hasFull) {
                                                                        next.delete(fullId)
                                                                        setSelectedChunkIds(next)
                                                                        setComposerBlocks((prev) =>
                                                                            prev.filter((b) => b.id !== fullId),
                                                                        )
                                                                    } else {
                                                                        next.add(fullId)
                                                                        setSelectedChunkIds(next)
                                                                        setComposerBlocks((prev) => [
                                                                            {
                                                                                id: fullId,
                                                                                type: 'chunk',
                                                                                sourceId: chunk.sourceId,
                                                                                label: chunk.filePath,
                                                                                text: chunk.fullText,
                                                                            },
                                                                            ...prev,
                                                                        ])
                                                                    }
                                                                }}
                                                                className="w-3.5 h-3.5 rounded border-[var(--border-color)] text-[var(--accent)] focus:ring-[var(--accent)] bg-[var(--bg-app)]"
                                                            />
                                                            <span className="text-[11px] text-[var(--text-secondary)] group-hover:text-[var(--text-primary)] transition-colors">Add full</span>
                                                        </label>
                                                    </div>
                                                </div>
                                            )
                                        })}
                                    </div>
                                </div>
                            </div>

                            {/* Right: Final Prompt Area */}
                            <div className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-lg p-5 flex flex-col h-full sticky top-0">
                                <div className="flex items-center justify-between mb-4 pb-2 border-b border-[var(--border-color)]">
                                    <div>
                                        <h4 className="text-sm font-semibold text-[var(--text-active)]">Final Prompt</h4>
                                        <span className="text-[10px] text-[var(--text-secondary)] uppercase tracking-tight">
                                            Ready to send to LLM
                                        </span>
                                    </div>
                                    <button
                                        type="button"
                                        className={`btn-primary px-3 py-1.5 flex items-center gap-1.5 transition-all ${copied ? 'bg-green-600 border-green-600' : ''}`}
                                        onClick={handleCopy}
                                        disabled={!composedPrompt}
                                    >
                                        {copied ? '✓' : '📋'} <span className="text-[10px]">{copied ? 'COPIED' : 'COPY'}</span>
                                    </button>
                                </div>
                                <div className="flex-1 min-h-[300px] bg-[var(--bg-app)] border border-[var(--border-color)] rounded-md p-4 overflow-auto font-mono text-xs leading-relaxed text-[var(--text-primary)] whitespace-pre-wrap selection:bg-[var(--accent)]/30">
                                    {composedPrompt || (
                                        <span className="text-[var(--text-secondary)] italic">No content selected yet. Use the checkboxes on the left to build your prompt.</span>
                                    )}
                                </div>
                                <div className="mt-4 pt-4 border-t border-[var(--border-color)] flex items-center justify-between">
                                    <button
                                        type="button"
                                        className="btn-outline px-3 py-1.5"
                                        onClick={() => {
                                            setComposerBlocks([])
                                            setSelectedChunkIds(new Set())
                                        }}
                                    >
                                        Reset selection
                                    </button>
                                    <button
                                        type="button"
                                        className={`btn-primary px-4 py-1.5 flex items-center gap-2 ${copied ? 'bg-green-600 border-green-600' : ''}`}
                                        onClick={handleCopy}
                                    >
                                        {copied ? '✓' : '📋'} {copied ? 'Copied Prompt' : 'Copy Prompt'}
                                    </button>
                                </div>
                            </div>
                        </section>
                    )}

                    {!result && !processing && !error && (
                        <div className="flex-1 flex flex-col items-center justify-center p-12 text-center">
                            <div className="text-4xl mb-4 opacity-20">✨</div>
                            <p className="text-[var(--text-secondary)] text-sm max-w-[300px] leading-relaxed">
                                Enter a query above to get an optimized prompt with context from your projects.
                            </p>
                        </div>
                    )}
                </div>

                {/* Right sidebar: Chat */}
                <div className="w-80 border-l border-[var(--border-color)] hidden xl:flex flex-col">
                    <Chat llmEnabled={appSettings?.llm_enabled ?? false} />
                </div>

                {/* Full-text popup modal */}
                {fullTextModal.open && fullTextModal.chunk && (
                    <div className="fixed inset-0 z-[100] flex items-center justify-center p-6 bg-black/60 backdrop-blur-sm" onClick={() => setFullTextModal({ open: false })}>
                        <div
                            className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-xl shadow-2xl w-full max-w-4xl max-h-full flex flex-col"
                            onClick={(e) => {
                                e.stopPropagation()
                            }}
                        >
                            <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--border-color)]">
                                <h4 className="text-sm font-semibold text-[var(--text-active)] truncate mr-4">{fullTextModal.chunk.filePath}</h4>
                                <button
                                    type="button"
                                    className="btn-outline px-3 py-1 text-[10px]"
                                    onClick={() => setFullTextModal({ open: false })}
                                >
                                    Close
                                </button>
                            </div>
                            <div className="flex-1 overflow-auto p-6 bg-[var(--bg-app)]">
                                <pre className="font-mono text-xs text-[var(--text-primary)] leading-relaxed">{fullTextModal.chunk.fullText}</pre>
                            </div>
                            <div className="flex justify-end items-center gap-3 px-6 py-4 border-t border-[var(--border-color)] bg-black/10 rounded-b-xl">
                                <button
                                    type="button"
                                    className="btn-outline px-4 py-2"
                                    onClick={() => {
                                        const c = fullTextModal.chunk!
                                        const newId = `path-${c.id}`
                                        if (!selectedChunkIds.has(newId)) {
                                            const next = new Set(selectedChunkIds)
                                            next.add(newId)
                                            setSelectedChunkIds(next)
                                            setComposerBlocks((prev) => [
                                                {
                                                    id: newId,
                                                    type: 'chunk',
                                                    sourceId: c.sourceId,
                                                    label: `Path: ${c.filePath}`,
                                                    text: c.filePath,
                                                },
                                                ...prev,
                                            ])
                                        }
                                    }}
                                >
                                    + Add path to prompt
                                </button>
                                <button
                                    type="button"
                                    className="btn-primary px-4 py-2 border-none"
                                    onClick={() => {
                                        const c = fullTextModal.chunk!
                                        if (!selectedChunkIds.has(c.id)) {
                                            const next = new Set(selectedChunkIds)
                                            next.add(c.id)
                                            setSelectedChunkIds(next)
                                            setComposerBlocks((prev) => [
                                                {
                                                    id: `chunk-${c.id}`,
                                                    type: 'chunk',
                                                    sourceId: c.sourceId,
                                                    label: c.filePath,
                                                    text: c.fullText,
                                                },
                                                ...prev,
                                            ])
                                        }
                                    }}
                                >
                                    + Add full text
                                </button>
                            </div>
                        </div>
                    </div>
                )}
            </div>
        </div >
    )
}
