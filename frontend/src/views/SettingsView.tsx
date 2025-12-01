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
    const isMountedRef = useRef(true)

    useEffect(() => {
        isMountedRef.current = true

        const loadSettingsWithRetry = async (attempt = 1) => {
            try {
                if (!isMountedRef.current) return
                setLoading(true)
                setMessage(null)
                const data = await getAppSettings()
                if (!isMountedRef.current) return
                setSettings(data)

                // Also check initial LLM status
                if (data.llm_enabled) {
                    const status = await getAppStatus()
                    if (!isMountedRef.current) return
                    updateLlmStatusFromAppStatus(status)
                }
            } catch (err) {
                console.error('Failed to load app settings (attempt', attempt, '):', err)
                if (!isMountedRef.current) return

                if (attempt < 5) {
                    // Soft message while we keep retrying
                    setMessage(`Loading settings (retry ${attempt + 1}/5)...`)
                    setTimeout(() => loadSettingsWithRetry(attempt + 1), 500 * attempt)
                } else {
                    setMessage('✗ Failed to load settings')
                }
            } finally {
                if (isMountedRef.current) {
                    setLoading(false)
                }
            }
        }

        loadSettingsWithRetry()

        // Cleanup polling on unmount
        return () => {
            isMountedRef.current = false
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

    const getLlmStatusBadge = () => {
        if (!settings?.llm_enabled) {
            return <span className="llm-status-badge disabled">Disabled</span>
        }
        if (llmStatus === 'ready') {
            return <span className="llm-status-badge ready">Ready</span>
        }
        if (llmStatus === 'error') {
            return <span className="llm-status-badge error">Error</span>
        }
        if (llmStatus === 'initializing') {
            return <span className="llm-status-badge initializing">Initializing...</span>
        }
        return <span className="llm-status-badge">Enabled</span>
    }

    return (
        <div className="view settings-view">
            {message && (
                <div className={`settings-toast ${message.startsWith('✓') ? 'success' : 'error'}`}>
                    {message}
                </div>
            )}

            <section className="settings-card">
                <div className="settings-card-header">
                    <span className="settings-icon">💾</span>
                    <h3>Data Storage</h3>
                </div>
                <div className="settings-card-body">
                    <div className="settings-row">
                        <span className="settings-row-label">Search index</span>
                        <span className="settings-row-value mono">~/Library/Application Support/Linggen/lancedb</span>
                    </div>
                    <div className="settings-row">
                        <span className="settings-row-label">Source metadata</span>
                        <span className="settings-row-value mono">~/Library/Application Support/Linggen/metadata.redb</span>
                    </div>
                </div>
            </section>

            <section className="settings-card">
                <div className="settings-card-header">
                    <span className="settings-icon">🔍</span>
                    <h3>Search Engine</h3>
                </div>
                <div className="settings-card-body">
                    <div className="settings-row">
                        <span className="settings-row-label">Embedding Model</span>
                        <span className="settings-row-value">all-MiniLM-L6-v2</span>
                    </div>
                    <div className="settings-row">
                        <span className="settings-row-label">Privacy</span>
                        <span className="settings-row-value highlight">100% local · offline-capable · data never leaves your device</span>
                    </div>
                </div>
            </section>

            <section className="settings-card">
                <div className="settings-card-header">
                    <span className="settings-icon">🤖</span>
                    <h3>Local LLM</h3>
                    <span className="settings-model-name">Qwen3-4B</span>
                </div>
                <div className="settings-card-body">
                    <div className="settings-row llm-toggle-row">
                        <div className="llm-toggle-left">
                            <label className="toggle-switch">
                                <input
                                    type="checkbox"
                                    checked={!!settings?.llm_enabled}
                                    onChange={handleToggleLlm}
                                    disabled={loading || saving || llmInitializing || !settings}
                                />
                                <span className="toggle-slider"></span>
                            </label>
                            <span className="settings-row-label">Enable Local LLM</span>
                        </div>
                        {getLlmStatusBadge()}
                    </div>

                    {llmInitializing && llmProgress && (
                        <div className="llm-progress-bar">
                            <div className="llm-progress-spinner"></div>
                            <span>{llmProgress}</span>
                        </div>
                    )}

                    {llmStatus === 'error' && !llmInitializing && llmProgress && (
                        <div className="llm-error-message">
                            <span>⚠️ {llmProgress}</span>
                        </div>
                    )}

                    <p className="settings-description">
                        Enables chat, profile generation, and AI-powered analysis. 
                        The model (~3GB) will be downloaded on first enable.
                    </p>
                </div>
            </section>

            <section className="settings-card danger">
                <div className="settings-card-header">
                    <span className="settings-icon">⚠️</span>
                    <h3>Danger Zone</h3>
                </div>
                <div className="settings-card-body">
                    <div className="danger-action">
                        <div className="danger-action-info">
                            <strong>Clear All Data</strong>
                            <p>Permanently delete all indexed data, sources, profiles, and settings. This cannot be undone.</p>
                        </div>
                        <button
                            type="button"
                            className="btn-danger"
                            onClick={handleClearAllData}
                            disabled={clearing}
                        >
                            {clearing ? 'Clearing...' : '🗑️ Clear All Data'}
                        </button>
                    </div>
                </div>
            </section>
        </div>
    )
}
