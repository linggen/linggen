import { useState, useEffect, useRef } from 'react'
import { getAppSettings, updateAppSettings, clearAllData, getAppStatus, retryInit, type AppSettings, type AppStatusResponse } from '../api'
import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'

export function SettingsView() {
    const [settings, setSettings] = useState<AppSettings | null>(null)
    const [loading, setLoading] = useState(false)
    const [saving, setSaving] = useState(false)
    const [message, setMessage] = useState<string | null>(null)
    const [clearing, setClearing] = useState(false)
    const [showClearConfirm, setShowClearConfirm] = useState(false)
    const [llmInitializing, setLlmInitializing] = useState(false)
    const [llmProgress, setLlmProgress] = useState<string | null>(null)
    const [llmStatus, setLlmStatus] = useState<'disabled' | 'initializing' | 'ready' | 'error'>('disabled')
    const progressPollRef = useRef<number | null>(null)
    const isMountedRef = useRef(true)
    
    // Update checker state
    const [checkingUpdate, setCheckingUpdate] = useState(false)
    const [updateAvailable, setUpdateAvailable] = useState(false)
    const [updateInfo, setUpdateInfo] = useState<{version?: string, date?: string, body?: string} | null>(null)
    const [downloading, setDownloading] = useState(false)
    const [downloadProgress, setDownloadProgress] = useState(0)

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
                    } catch {
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

    const handleCheckForUpdates = async () => {
        setCheckingUpdate(true)
        setMessage(null)
        try {
            const update = await check()
            if (update) {
                setUpdateAvailable(true)
                setUpdateInfo({
                    version: update.version,
                    date: update.date,
                    body: update.body
                })
                setMessage(`✓ Update available: v${update.version}`)
            } else {
                setUpdateAvailable(false)
                setUpdateInfo(null)
                setMessage('✓ You are running the latest version')
            }
        } catch (err) {
            console.error('Update check failed:', err)
            setMessage(`✗ Failed to check for updates: ${err}`)
        } finally {
            setCheckingUpdate(false)
            setTimeout(() => setMessage(null), 5000)
        }
    }

    const handleInstallUpdate = async () => {
        if (!updateInfo) return
        setDownloading(true)
        setMessage(null)
        try {
            const update = await check()
            if (update) {
                // Download and install
                await update.downloadAndInstall((event) => {
                    switch (event.event) {
                        case 'Started':
                            setDownloadProgress(0)
                            setMessage('Downloading update...')
                            break
                        case 'Progress':
                            // Increment progress (indeterminate since we don't have total size)
                            setDownloadProgress((prev) => Math.min(prev + 5, 95))
                            break
                        case 'Finished':
                            setDownloadProgress(100)
                            setMessage('✓ Update installed! Restarting...')
                            break
                    }
                })
                
                // Relaunch the app
                await relaunch()
            }
        } catch (err) {
            console.error('Update installation failed:', err)
            setMessage(`✗ Failed to install update: ${err}`)
            setDownloading(false)
        }
    }

    const handleClearAllData = () => {
        setShowClearConfirm(true)
    }

    const confirmClearAllData = async () => {
        setShowClearConfirm(false)
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

            <section className="settings-card">
                <div className="settings-card-header">
                    <span className="settings-icon">📊</span>
                    <h3>Analytics</h3>
                </div>
                <div className="settings-card-body">
                    <div className="settings-row llm-toggle-row">
                        <div className="llm-toggle-left">
                            <label className="toggle-switch">
                                <input
                                    type="checkbox"
                                    checked={settings?.analytics_enabled ?? true}
                                    onChange={async () => {
                                        if (!settings || saving) return
                                        const next = { ...settings, analytics_enabled: !settings.analytics_enabled }
                                        setSettings(next)
                                        setSaving(true)
                                        try {
                                            await updateAppSettings(next)
                                            setMessage('✓ Settings saved')
                                        } catch (err) {
                                            console.error('Failed to save settings:', err)
                                            setMessage('✗ Failed to save settings')
                                        } finally {
                                            setSaving(false)
                                            setTimeout(() => setMessage(null), 3000)
                                        }
                                    }}
                                    disabled={loading || saving || !settings}
                                />
                                <span className="toggle-slider"></span>
                            </label>
                            <span className="settings-row-label">Help improve Linggen</span>
                        </div>
                        <span className={`llm-status-badge ${settings?.analytics_enabled !== false ? 'ready' : 'disabled'}`}>
                            {settings?.analytics_enabled !== false ? 'Enabled' : 'Disabled'}
                        </span>
                    </div>
                    <p className="settings-description">
                        Send anonymous usage statistics to help improve Linggen. 
                        We only collect basic usage data (app launches, sources added) — <strong>no code content, file paths, or personal information is ever sent</strong>.
                    </p>
                </div>
            </section>

            <section className="settings-card">
                <div className="settings-card-header">
                    <span className="settings-icon">🔄</span>
                    <h3>Software Update</h3>
                    <span className="settings-model-name">v0.5.0</span>
                </div>
                <div className="settings-card-body">
                    <div className="settings-row">
                        <div className="danger-action-info" style={{ flex: 1 }}>
                            <strong>Application Updates</strong>
                            <p style={{ margin: '4px 0 0 0', fontSize: '12px', color: 'var(--text-secondary)' }}>
                                Check for new versions and install updates automatically.
                            </p>
                        </div>
                        <button
                            type="button"
                            className="btn-action"
                            onClick={handleCheckForUpdates}
                            disabled={checkingUpdate || downloading}
                            style={{ whiteSpace: 'nowrap' }}
                        >
                            {checkingUpdate ? 'Checking...' : 'Check for Updates'}
                        </button>
                    </div>

                    {updateAvailable && updateInfo && (
                        <div style={{
                            marginTop: '12px',
                            padding: '12px 14px',
                            background: 'rgba(96, 165, 250, 0.1)',
                            border: '1px solid rgba(96, 165, 250, 0.2)',
                            borderRadius: '6px'
                        }}>
                            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '8px' }}>
                                <div>
                                    <strong style={{ color: '#60a5fa', fontSize: '13px' }}>
                                        Version {updateInfo.version} Available
                                    </strong>
                                    {updateInfo.date && (
                                        <div style={{ fontSize: '11px', color: 'var(--text-secondary)', marginTop: '2px' }}>
                                            Released: {new Date(updateInfo.date).toLocaleDateString()}
                                        </div>
                                    )}
                                </div>
                                <button
                                    type="button"
                                    className="btn-action"
                                    onClick={handleInstallUpdate}
                                    disabled={downloading}
                                    style={{ background: '#60a5fa', borderColor: '#60a5fa' }}
                                >
                                    {downloading ? 'Installing...' : 'Install Update'}
                                </button>
                            </div>
                            {updateInfo.body && (
                                <div style={{
                                    fontSize: '12px',
                                    color: 'var(--text-secondary)',
                                    marginTop: '8px',
                                    paddingTop: '8px',
                                    borderTop: '1px solid rgba(96, 165, 250, 0.2)',
                                    maxHeight: '150px',
                                    overflowY: 'auto'
                                }}>
                                    <strong>What's New:</strong>
                                    <pre style={{ 
                                        whiteSpace: 'pre-wrap', 
                                        fontFamily: 'inherit',
                                        margin: '4px 0 0 0'
                                    }}>{updateInfo.body}</pre>
                                </div>
                            )}
                            {downloading && downloadProgress > 0 && (
                                <div style={{ marginTop: '8px' }}>
                                    <div style={{
                                        width: '100%',
                                        height: '4px',
                                        background: 'rgba(96, 165, 250, 0.2)',
                                        borderRadius: '2px',
                                        overflow: 'hidden'
                                    }}>
                                        <div style={{
                                            width: `${downloadProgress}%`,
                                            height: '100%',
                                            background: '#60a5fa',
                                            transition: 'width 0.3s ease'
                                        }} />
                                    </div>
                                    <div style={{ fontSize: '11px', color: 'var(--text-secondary)', marginTop: '4px' }}>
                                        Downloading... {downloadProgress}%
                                    </div>
                                </div>
                            )}
                        </div>
                    )}

                    <p className="settings-description" style={{ marginTop: '12px' }}>
                        Updates are downloaded from the official Linggen releases repository. 
                        The app will restart automatically after installation.
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

            {/* Clear Data Confirmation Modal */}
            {showClearConfirm && (
                <div className="modal-overlay" onClick={() => setShowClearConfirm(false)}>
                    <div className="modal-content" onClick={e => e.stopPropagation()}>
                        <div className="modal-header">
                            <h3>⚠️ Clear All Data</h3>
                        </div>
                        <div className="modal-body">
                            <p style={{ marginBottom: '1rem', color: 'var(--text)' }}>
                                This will <strong>permanently delete</strong>:
                            </p>
                            <ul style={{ margin: '0 0 1rem 1.5rem', color: 'var(--text-muted)', lineHeight: '1.8' }}>
                                <li>All indexed chunks in vector database</li>
                                <li>All source configurations</li>
                                <li>All project profiles</li>
                                <li>All indexing history</li>
                                <li>All uploaded files</li>
                            </ul>
                            <p style={{ marginBottom: '0.5rem', color: 'var(--text-muted)', fontSize: '0.85rem' }}>
                                ✓ Your settings and downloaded models will be preserved.
                            </p>
                            <p style={{ color: '#ef4444', fontWeight: '600' }}>
                                This action cannot be undone!
                            </p>
                        </div>
                        <div className="modal-footer">
                            <button
                                type="button"
                                className="btn-secondary"
                                onClick={() => setShowClearConfirm(false)}
                            >
                                Cancel
                            </button>
                            <button
                                type="button"
                                className="btn-danger"
                                onClick={confirmClearAllData}
                            >
                                🗑️ Yes, Delete Everything
                            </button>
                        </div>
                    </div>
                </div>
            )}
        </div>
    )
}
