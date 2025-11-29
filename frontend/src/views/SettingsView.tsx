import { useState, useEffect, useRef } from 'react'
import { getAppSettings, updateAppSettings, clearAllData, getAppStatus, retryInit, type AppSettings, type AppStatusResponse } from '../api'

export function SettingsView() {
    const [settings, setSettings] = useState<AppSettings | null>(null)
    const [loading, setLoading] = useState(false)
    const [saving, setSaving] = useState(false)
    const [message, setMessage] = useState<string | null>(null)
    const [clearing, setClearing] = useState(false)
    const [llmInitializing, setLlmInitializing] = useState(false)
    const [llmProgress, setLlmProgress] = useState<string | null>(null)
    const [llmStatus, setLlmStatus] = useState<'disabled' | 'initializing' | 'ready' | 'error'>('disabled')
    const progressPollRef = useRef<number | null>(null)

    useEffect(() => {
        const loadSettings = async () => {
            try {
                setLoading(true)
                const data = await getAppSettings()
                setSettings(data)
                
                // Also check initial LLM status
                if (data.llm_enabled) {
                    const status = await getAppStatus()
                    updateLlmStatusFromAppStatus(status)
                }
            } catch (err) {
                console.error('Failed to load app settings:', err)
                setMessage('✗ Failed to load settings')
            } finally {
                setLoading(false)
            }
        }
        loadSettings()

        // Cleanup polling on unmount
        return () => {
            if (progressPollRef.current) {
                clearInterval(progressPollRef.current)
            }
        }
    }, [])

    const updateLlmStatusFromAppStatus = (status: AppStatusResponse) => {
        if (status.status === 'ready') {
            setLlmStatus('ready')
            setLlmProgress(null)
            setLlmInitializing(false)
        } else if (status.status === 'error') {
            setLlmStatus('error')
            setLlmProgress(status.message || 'Error')
            setLlmInitializing(false)
        } else if (status.status === 'initializing') {
            setLlmStatus('initializing')
            setLlmProgress(status.progress || 'Initializing...')
            setLlmInitializing(true)
        }
    }

    const startProgressPolling = () => {
        // Poll every 1 second for progress updates
        progressPollRef.current = window.setInterval(async () => {
            try {
                const status = await getAppStatus()
                updateLlmStatusFromAppStatus(status)
                
                // Stop polling when done
                if (status.status === 'ready' || status.status === 'error') {
                    if (progressPollRef.current) {
                        clearInterval(progressPollRef.current)
                        progressPollRef.current = null
                    }
                }
            } catch (err) {
                console.error('Failed to poll status:', err)
            }
        }, 1000)
    }

    const handleToggleLlm = async () => {
        if (!settings || saving) return
        const newEnabled = !settings.llm_enabled
        const next = { ...settings, llm_enabled: newEnabled }
        setSettings(next)
        setSaving(true)
        setMessage(null)
        
        try {
            await updateAppSettings(next)
            
            if (newEnabled) {
                // When enabling, trigger model initialization
                setLlmInitializing(true)
                setLlmStatus('initializing')
                setLlmProgress('Initializing LLM...')
                
                const initResult = await retryInit()
                if (initResult.success) {
                    // Immediately check status to get actual progress
                    try {
                        const status = await getAppStatus()
                        updateLlmStatusFromAppStatus(status)
                    } catch (e) {
                        // Ignore, polling will catch up
                    }
                    startProgressPolling()
                } else {
                    setLlmStatus('error')
                    setLlmProgress(initResult.message)
                    setLlmInitializing(false)
                }
            } else {
                // When disabling, just update status
                setLlmStatus('disabled')
                setLlmProgress(null)
                setLlmInitializing(false)
                if (progressPollRef.current) {
                    clearInterval(progressPollRef.current)
                    progressPollRef.current = null
                }
            }
            
            setMessage('✓ Settings saved')
        } catch (err) {
            console.error('Failed to save settings:', err)
            setMessage('✗ Failed to save settings')
        } finally {
            setSaving(false)
            setTimeout(() => setMessage(null), 3000)
        }
    }

    const handleClearAllData = async () => {
        const confirmed = window.confirm(
            '⚠️ WARNING: This will permanently delete ALL indexed data, sources, and settings.\n\n' +
            'This includes:\n' +
            '• All indexed chunks in LanceDB\n' +
            '• All source configurations\n' +
            '• All profiles\n' +
            '• All indexing history\n\n' +
            'This action CANNOT be undone!\n\n' +
            'Are you sure you want to continue?'
        )

        if (!confirmed) return

        setClearing(true)
        setMessage(null)
        try {
            await clearAllData()
            setMessage('✓ All data cleared successfully. Refreshing page...')
            setTimeout(() => {
                window.location.reload()
            }, 1500)
        } catch (err) {
            console.error('Failed to clear data:', err)
            setMessage(`✗ Failed to clear data: ${err}`)
        } finally {
            setClearing(false)
        }
    }

    return (
        <div className="view">
            <section className="section settings-section">
                <div className="settings-group">
                    <h3>Data Storage</h3>
                    <div className="settings-item">
                        <span className="settings-label">Search index</span>
                        <span className="settings-value">./backend/data/lancedb</span>
                    </div>
                    <div className="settings-item">
                        <span className="settings-label">Source metadata</span>
                        <span className="settings-value">./backend/data/metadata.redb</span>
                    </div>
                </div>

                <div className="settings-group">
                    <h3>Search Engine</h3>
                    <div className="settings-item">
                        <span className="settings-label">Embedding Model</span>
                        <span className="settings-value">all-MiniLM-L6-v2</span>
                    </div>
                    <div className="settings-item">
                        <span className="settings-label">Privacy</span>
                        <span className="settings-value">100% local, offline-capable, your data never leaves your device</span>
                    </div>
                </div>

                <div className="settings-group">
                    <h3>Local LLM (Qwen3-4B)</h3>
                    {message && (
                        <div className={`status ${message.startsWith('✓') ? 'success' : 'error'}`} style={{ marginBottom: '0.75rem' }}>
                            {message}
                        </div>
                    )}
                    <div className="settings-item">
                        <span className="settings-label">Enable Local LLM</span>
                        <span className="settings-value">
                            <label style={{ display: 'inline-flex', alignItems: 'center', gap: '0.5rem', cursor: 'pointer' }}>
                                <input
                                    type="checkbox"
                                    checked={!!settings?.llm_enabled}
                                    onChange={handleToggleLlm}
                                    disabled={loading || saving || llmInitializing || !settings}
                                />
                                <span>
                                    {settings?.llm_enabled 
                                        ? (llmStatus === 'ready' ? 'Enabled (Ready)' : llmStatus === 'error' ? 'Enabled (Error)' : 'Enabled')
                                        : 'Disabled'}
                                </span>
                            </label>
                        </span>
                    </div>
                    {llmInitializing && llmProgress && (
                        <div className="settings-item" style={{ 
                            background: 'var(--bg-tertiary)', 
                            borderRadius: '8px', 
                            padding: '0.75rem',
                            border: '1px solid var(--border-color)'
                        }}>
                            <div style={{ display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
                                <span style={{ 
                                    display: 'inline-block', 
                                    width: '12px', 
                                    height: '12px', 
                                    border: '2px solid var(--accent)', 
                                    borderTopColor: 'transparent',
                                    borderRadius: '50%',
                                    animation: 'spin 1s linear infinite'
                                }} />
                                <span style={{ color: 'var(--text-muted)' }}>{llmProgress}</span>
                            </div>
                        </div>
                    )}
                    {llmStatus === 'error' && !llmInitializing && llmProgress && (
                        <div className="settings-item" style={{ 
                            background: 'rgba(239, 68, 68, 0.1)', 
                            borderRadius: '8px', 
                            padding: '0.75rem',
                            border: '1px solid var(--error)'
                        }}>
                            <span style={{ color: 'var(--error)' }}>⚠️ {llmProgress}</span>
                        </div>
                    )}
                    <div className="settings-item settings-item-muted">
                        <span>
                            The local Qwen3-4B LLM enables features like chat, profile generation, and AI-powered analysis. 
                            When disabled, these features will not be available. The model (~3GB) will be downloaded when you first enable it.
                        </span>
                    </div>
                </div>

                <div className="settings-group">
                    <h3>Danger Zone</h3>
                    <div className="settings-item">
                        <div style={{ display: 'flex', flexDirection: 'column', gap: '0.5rem', width: '100%' }}>
                            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                                <div>
                                    <div style={{ fontWeight: 600, marginBottom: '0.25rem' }}>Clear All Data</div>
                                    <div style={{ fontSize: '0.85rem', color: 'var(--text-muted)' }}>
                                        Permanently delete all indexed data, sources, profiles, and settings. This cannot be undone.
                                    </div>
                                </div>
                                <button
                                    type="button"
                                    onClick={handleClearAllData}
                                    disabled={clearing}
                                    style={{
                                        padding: '0.5rem 1rem',
                                        background: 'var(--error)',
                                        color: 'white',
                                        border: 'none',
                                        borderRadius: '6px',
                                        cursor: clearing ? 'not-allowed' : 'pointer',
                                        fontSize: '0.9rem',
                                        fontWeight: '500',
                                        opacity: clearing ? 0.5 : 1,
                                        whiteSpace: 'nowrap',
                                    }}
                                >
                                    {clearing ? 'Clearing...' : '🗑️ Clear All Data'}
                                </button>
                            </div>
                        </div>
                    </div>
                </div>
            </section>
        </div>
    )
}
