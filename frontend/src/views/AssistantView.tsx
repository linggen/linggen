import React, { useState, useEffect } from 'react'
import { enhancePrompt, getProfile, listResources, type EnhancedPromptResponse, type Resource, type IntentType, type SourceProfile as SourceProfileMeta } from '../api'
import { Chat } from '../components/Chat'

type ComposerBlockType = 'profile' | 'chunk' | 'question' | 'path'

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
    const [sources, setSources] = useState<Resource[]>([])
    const [profilesBySource, setProfilesBySource] = useState<Record<string, SourceProfileMeta>>({})
    const [includedProfileSourceIds, setIncludedProfileSourceIds] = useState<Set<string>>(new Set())
    const [loadingProfiles, setLoadingProfiles] = useState(false)

    useEffect(() => {
        const loadSourcesAndProfiles = async () => {
            try {
                setLoadingProfiles(true)
                const res = await listResources()
                setSources(res.resources)

                const entries = await Promise.all(
                    res.resources.map(async (src) => {
                        try {
                            const profile = await getProfile(src.id)
                            return [src.id, profile] as const
                        } catch {
                            return null
                        }
                    }),
                )

                const map: Record<string, SourceProfileMeta> = {}
                for (const entry of entries) {
                    if (entry) {
                        const [id, profile] = entry
                        map[id] = profile
                    }
                }
                setProfilesBySource(map)
            } finally {
                setLoadingProfiles(false)
            }
        }

        loadSourcesAndProfiles()
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
        <div className="view">
            <div className="assistant-layout">
                {/* Left + middle + right columns */}
                <div className="assistant-main-col">
                    {/* Query Input */}
                    <section className="section">
                        <form onSubmit={handleEnhance}>
                            <div className="form-group">
                                <label htmlFor="query">Your Query</label>
                                <textarea
                                    id="query"
                                    value={query}
                                    onChange={(e) => setQuery(e.target.value)}
                                    placeholder="e.g., 'Fix the timeout bug in auth service' or 'Explain how the login function works'"
                                    rows={3}
                                    required
                                />
                            </div>
                            <button type="submit" disabled={processing}>
                                {processing ? '✨ Enhancing...' : '✨ Enhance Prompt'}
                            </button>
                        </form>
                    </section>

                    {error && <div className="status error">{error}</div>}

                    {/* Two-column composer layout */}
                    {result && (
                        <section className="section assistant-composer-two-col">
                            {/* Left: Sources & Profiles + Context Chunks */}
                            <div className="assistant-col left-col">
                                <h4>Sources & Profiles</h4>
                                <div className="small-text">
                                    Intent: {formatIntent(result.intent)}
                                </div>
                                <div className="small-text">
                                    Retrieved chunks: {availableChunks.length}
                                </div>
                                {loadingProfiles && (
                                    <div className="small-text">Loading profiles…</div>
                                )}
                                <div className="sources-list">
                                    {sources.map((src) => {
                                        const profile = profilesBySource[src.id]
                                        const included = includedProfileSourceIds.has(src.id)
                                        const snippet =
                                            profile?.description?.slice(0, 160) ||
                                            'No profile yet. You can generate one from the Sources view.'
                                        return (
                                            <div key={src.id} className="source-card">
                                                <div className="source-card-header">
                                                    <span className="source-name">{src.name}</span>
                                                    <span className="source-type-pill">
                                                        {src.resource_type.toUpperCase()}
                                                    </span>
                                                </div>
                                                <div className="source-path small-text">{src.path}</div>
                                                <div className="source-snippet small-text">{snippet}</div>
                                                <label className="source-include-toggle">
                                                    <input
                                                        type="checkbox"
                                                        checked={included}
                                                        onChange={() => {
                                                            const next = new Set(includedProfileSourceIds)
                                                            if (included) {
                                                                next.delete(src.id)
                                                                setIncludedProfileSourceIds(next)
                                                                setComposerBlocks((prev) =>
                                                                    prev.filter(
                                                                        (b) =>
                                                                            !(
                                                                                b.type === 'profile' &&
                                                                                b.sourceId === src.id
                                                                            ),
                                                                    ),
                                                                )
                                                            } else {
                                                                next.add(src.id)
                                                                setIncludedProfileSourceIds(next)
                                                                const text = profile?.description || ''
                                                                const block: ComposerBlock = {
                                                                    id: `profile-${src.id}`,
                                                                    type: 'profile',
                                                                    sourceId: src.id,
                                                                    label: `Profile: ${src.name}`,
                                                                    text,
                                                                }
                                                                setComposerBlocks((prev) => [block, ...prev])
                                                            }
                                                        }}
                                                    />
                                                    <span>Include profile in prompt</span>
                                                </label>
                                            </div>
                                        )
                                    })}
                                    {sources.length === 0 && !loadingProfiles && (
                                        <p className="muted small-text">
                                            No sources yet. Add and index a source to generate profiles.
                                        </p>
                                    )}
                                </div>

                                {/* Context Chunks in same column */}
                                <div className="chunks-section">
                                    <div className="chunks-header">
                                        <h4>Context Chunks</h4>
                                        <div className="chunks-controls">
                                            <span className="small-text">Top K:</span>
                                            <select
                                                value={chunkLimit}
                                                onChange={(e) => setChunkLimit(parseInt(e.target.value, 10))}
                                            >
                                                <option value={3}>3</option>
                                                <option value={5}>5</option>
                                                <option value={8}>8</option>
                                                <option value={10}>10</option>
                                            </select>
                                        </div>
                                    </div>
                                    {availableChunks.length === 0 && (
                                        <div className="empty-state small">
                                            No context chunks retrieved yet. Try a different query or
                                            strategy.
                                        </div>
                                    )}
                                    <div className="chunks-list">
                                        {availableChunks.slice(0, chunkLimit).map((chunk) => {
                                            const pathId = `path-${chunk.id}`
                                            const fullId = `full-${chunk.id}`
                                            const hasPath = selectedChunkIds.has(pathId)
                                            const hasFull = selectedChunkIds.has(fullId)
                                            const isSelected = hasPath || hasFull

                                            return (
                                                <div
                                                    key={chunk.id}
                                                    className={`chunk-card ${isSelected ? 'selected' : ''}`}
                                                >
                                                    <div className="chunk-card-header">
                                                        <a
                                                            href="#"
                                                            className="chunk-file-link"
                                                            onClick={(e) => {
                                                                e.preventDefault()
                                                                setFullTextModal({ open: true, chunk })
                                                            }}
                                                        >
                                                            {chunk.filePath}
                                                        </a>
                                                    </div>

                                                    <div className="chunk-actions">
                                                        <label className="checkbox-label">
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
                                                            />
                                                            <span>Add path</span>
                                                        </label>

                                                        <label className="checkbox-label">
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
                                                            />
                                                            <span>Add full</span>
                                                        </label>
                                                    </div>
                                                </div>
                                            )
                                        })}
                                    </div>
                                </div>
                            </div>

                            {/* Right: Final Prompt */}
                            <div className="assistant-col right-col">
                                <div className="composer-header">
                                    <div>
                                        <h4>Final Prompt</h4>
                                        <span className="small-text">
                                            This is what you can send to your LLM.
                                        </span>
                                    </div>
                                    <button
                                        type="button"
                                        className={`copy-btn compact ${copied ? 'copied' : ''}`}
                                        onClick={handleCopy}
                                        disabled={!composedPrompt}
                                    >
                                        {copied ? '✓ Copied!' : '📋 Copy'}
                                    </button>
                                </div>
                                <div className="composer-preview">
                                    <div className="code-block">
                                        {composedPrompt || 'No content selected yet.'}
                                    </div>
                                </div>
                                <div className="composer-actions">
                                    <button
                                        type="button"
                                        className="text-btn"
                                        onClick={() => {
                                            setComposerBlocks([])
                                            setSelectedChunkIds(new Set())
                                            setIncludedProfileSourceIds(new Set())
                                        }}
                                    >
                                        Reset selection
                                    </button>
                                    <button
                                        type="button"
                                        className={`copy-btn ${copied ? 'copied' : ''}`}
                                        onClick={handleCopy}
                                    >
                                        {copied ? '✓ Copied!' : '📋 Copy Prompt'}
                                    </button>
                                </div>
                            </div>
                        </section>
                    )}

                    {!result && !processing && !error && (
                        <div className="empty-state">
                            Enter a query above to get an optimized prompt with context.
                        </div>
                    )}
                </div>

                <div className="assistant-sidebar-col">
                    <Chat />
                </div>

                {/* Full-text popup modal */}
                {fullTextModal.open && fullTextModal.chunk && (
                    <div className="modal-backdrop" onClick={() => setFullTextModal({ open: false })}>
                        <div
                            className="modal"
                            onClick={(e) => {
                                e.stopPropagation()
                            }}
                        >
                            <div className="modal-header">
                                <h4>{fullTextModal.chunk.filePath}</h4>
                                <button
                                    type="button"
                                    className="text-btn"
                                    onClick={() => setFullTextModal({ open: false })}
                                >
                                    Close
                                </button>
                            </div>
                            <div className="modal-body">
                                <pre className="code-block">{fullTextModal.chunk.fullText}</pre>
                            </div>
                            <div className="modal-footer">
                                <button
                                    type="button"
                                    className="text-btn"
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
                                    className="text-btn"
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
