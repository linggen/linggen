/* eslint-disable @typescript-eslint/no-unused-vars */
import { useState, useEffect, useRef } from 'react'
import { getAppSettings, updateAppSettings, clearAllData, getAppStatus, retryInit, type AppSettings, type AppStatusResponse } from '../api'
import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { getVersion } from '@tauri-apps/api/app'
import { Command } from '@tauri-apps/plugin-shell'

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

    // Only show updater UI when running inside the Tauri WebView (not a normal browser tab).
    const isTauriApp =
        typeof window !== 'undefined' &&
        (!!(window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ ||
            !!(window as unknown as { __TAURI__?: unknown }).__TAURI__)

    // App version (for Software Update section)
    const [appVersion, setAppVersion] = useState<string | null>(null)

    // Avoid TypeScript unused warnings while Local LLM UI is hidden.
    // These values and helpers are kept for future Local LLM support.
    void llmInitializing
    void llmProgress

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

    // Load app version from Tauri at startup
    useEffect(() => {
        if (!isTauriApp) return
        getVersion()
            .then((ver) => setAppVersion(ver))
            .catch(() => {
                // Ignore errors; we simply won't show the version badge
            })
    }, [isTauriApp])

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
        if (!isTauriApp) {
            setMessage('✗ Updates are only available inside the desktop app')
            setTimeout(() => setMessage(null), 5000)
            return
        }
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

    const extractErrorText = (e: unknown) => {
        const msg = (() => {
            if (typeof e === 'object' && e !== null && 'message' in e) {
                const m = (e as { message?: unknown }).message
                if (typeof m === 'string') return m
            }
            return ''
        })()

        let json = ''
        try {
            json = JSON.stringify(e)
        } catch {
            // ignore stringify errors (e.g. circular)
        }

        const text = `${msg} ${String(e)} ${json}`.trim()
        return { msg, json, text, lower: text.toLowerCase() }
    }

    const handleInstallUpdate = async () => {
        if (!updateInfo) return
        if (!isTauriApp) {
            setMessage('✗ Updates are only available inside the desktop app')
            setTimeout(() => setMessage(null), 5000)
            return
        }
        setDownloading(true)
        setMessage(null)
        setDownloadProgress(0)
        
        try {
            const update = await check()
            if (!update) {
                setMessage('✗ Update no longer available')
                setDownloading(false)
                setDownloadProgress(0)
                setUpdateAvailable(false)
                return
            }

            // Download and install app update with retry logic for network errors
            const maxRetries = 5
            let lastError: unknown | null = null
            let installSuccess = false
            
            for (let attempt = 0; attempt < maxRetries; attempt++) {
                try {
                    await update.downloadAndInstall((event) => {
                        switch (event.event) {
                            case 'Started':
                                setDownloadProgress(0)
                                if (attempt > 0) {
                                    setMessage(`Downloading app update... (retry ${attempt + 1}/${maxRetries})`)
                                } else {
                                    setMessage('Downloading app update...')
                                }
                                break
                            case 'Progress':
                                // Increment progress (indeterminate since we don't have total size)
                                setDownloadProgress((prev) => Math.min(prev + 5, 90))
                                break
                            case 'Finished':
                                setDownloadProgress(90)
                                setMessage('✓ App update installed! Updating CLI...')
                                installSuccess = true
                                break
                        }
                    })
                    // Success - break out of retry loop
                    installSuccess = true
                    break
                } catch (err: unknown) {
                    lastError = err
                    const { msg: errorMsg, text: errorText, lower: errorStr } = extractErrorText(err)
                    
                    // Categorize error types
                    const isNetworkError = errorStr.includes('504') || 
                                          errorStr.includes('503') || 
                                          errorStr.includes('gateway timeout') ||
                                          errorStr.includes('service unavailable') ||
                                          errorStr.includes('timeout') ||
                                          errorStr.includes('network') ||
                                          errorStr.includes('connection') ||
                                          errorStr.includes('econnrefused') ||
                                          errorStr.includes('enotfound')
                    
                    const isSignatureError = errorStr.includes('signature') ||
                                            errorStr.includes('minisign') ||
                                            errorStr.includes('invalid encoding') ||
                                            errorStr.includes('verification failed')
                    
                    const isPermissionError = errorStr.includes('permission') ||
                                             errorStr.includes('eacces') ||
                                             errorStr.includes('access denied') ||
                                             errorStr.includes('unauthorized')
                    
                    const isDiskError = errorStr.includes('disk') ||
                                      errorStr.includes('space') ||
                                      errorStr.includes('enospc') ||
                                      errorStr.includes('no space')
                    
                    // Always log the raw error: updater errors often stringify poorly.
                    console.error('Update install attempt failed:', {
                        attempt: attempt + 1,
                        maxRetries,
                        errorMsg,
                        errorText,
                        err
                    })

                    // Handle non-retryable errors immediately
                    if (isSignatureError) {
                        const devDetail = import.meta.env.DEV && errorText ? ` Details: ${errorText}` : ''
                        setMessage(`✗ Signature verification failed. The update may be corrupted or from an untrusted source. Please try again later or download manually.${devDetail}`)
                        setDownloading(false)
                        setDownloadProgress(0)
                        return
                    }
                    
                    if (isPermissionError) {
                        setMessage(`✗ Permission denied. Please ensure Linggen has write permissions to install updates.`)
                        setDownloading(false)
                        setDownloadProgress(0)
                        return
                    }
                    
                    if (isDiskError) {
                        setMessage(`✗ Insufficient disk space. Please free up space and try again.`)
                        setDownloading(false)
                        setDownloadProgress(0)
                        return
                    }
                    
                    // Retry network errors
                    if (isNetworkError && attempt < maxRetries - 1) {
                        const delay = Math.min(1000 * Math.pow(2, attempt), 8000) // 1s, 2s, 4s, 8s, 8s
                        setMessage(`⚠️ Download failed (network error), retrying in ${delay/1000}s... (attempt ${attempt + 1}/${maxRetries})`)
                        setDownloadProgress(0) // Reset progress on retry
                        await new Promise(resolve => setTimeout(resolve, delay))
                        continue
                    } else {
                        // Not retryable or max retries reached
                        throw err
                    }
                }
            }
            
            if (!installSuccess && lastError) {
                const { msg: errorMsg, text: errorText, lower: errorStr } = extractErrorText(lastError)

                console.error('Update install failed after retries:', {
                    errorMsg,
                    errorText,
                    lastError
                })
                
                // Provide specific error messages based on error type
                if (errorStr.includes('signature') || errorStr.includes('minisign') || errorStr.includes('invalid encoding')) {
                    setMessage(`✗ Signature verification failed. The update may be corrupted. Please try again later.`)
                } else if (errorStr.includes('network') || errorStr.includes('timeout') || errorStr.includes('504') || errorStr.includes('503')) {
                    setMessage(`✗ Network error: ${errorMsg || 'request failed'}. Please check your internet connection and try again.`)
                } else if (errorStr.includes('permission') || errorStr.includes('access')) {
                    setMessage(`✗ Permission denied. Please ensure Linggen has write permissions.`)
                } else if (errorStr.includes('disk') || errorStr.includes('space')) {
                    setMessage(`✗ Insufficient disk space. Please free up space and try again.`)
                } else {
                    setMessage(`✗ Failed to install update: ${errorMsg || errorText || 'Unknown error'}`)
                }
                
                setDownloading(false)
                setDownloadProgress(0)
                return
            }
            
            // Also update CLI if available
            try {
                setDownloadProgress(95)
                setMessage('Updating CLI...')
                // Execute 'linggen update' command to update CLI
                const cliUpdate = Command.create('linggen', ['update'])
                const cliResult = await cliUpdate.execute()
                
                if (cliResult.code === 0) {
                    setMessage('✓ App and CLI updated! Restarting...')
                } else {
                    // CLI update failed with non-zero exit code
                    console.warn('CLI update failed with exit code:', cliResult.code)
                    setMessage('✓ App updated! (CLI update failed) Restarting...')
                }
            } catch (cliErr: unknown) {
                // CLI update failed, but app update succeeded - continue anyway
                const { text: cliErrorMsg } = extractErrorText(cliErr)
                console.warn('CLI update failed (may not be installed):', cliErr)
                
                // Check if CLI is not installed (common case)
                if (cliErrorMsg.includes('not found') || cliErrorMsg.includes('command not found') || cliErrorMsg.includes('enoent')) {
                    setMessage('✓ App updated! (CLI not installed) Restarting...')
                } else {
                    setMessage('✓ App updated! (CLI update skipped) Restarting...')
                }
            }
            
            setDownloadProgress(100)
            
            // Small delay to show final message
            await new Promise(resolve => setTimeout(resolve, 1000))
            
            // Relaunch the app
            try {
                await relaunch()
            } catch (relaunchErr) {
                console.error('Failed to relaunch app:', relaunchErr)
                setMessage('✓ Update installed! Please restart the app manually.')
                setDownloading(false)
                // Don't reset progress here - show success
            }
        } catch (err: unknown) {
            console.error('Update installation failed:', err)
            const { msg: errorMsg, lower: errorStr } = extractErrorText(err)
            
            // Provide user-friendly error messages
            if (errorStr.includes('signature') || errorStr.includes('minisign') || errorStr.includes('invalid encoding')) {
                setMessage('✗ Signature verification failed. The update may be corrupted or from an untrusted source.')
            } else if (errorStr.includes('network') || errorStr.includes('timeout') || errorStr.includes('504') || errorStr.includes('503')) {
                setMessage(`✗ Network error: ${errorMsg}. Please check your internet connection and try again.`)
            } else if (errorStr.includes('permission') || errorStr.includes('access')) {
                setMessage('✗ Permission denied. Please ensure Linggen has write permissions to install updates.')
            } else if (errorStr.includes('disk') || errorStr.includes('space')) {
                setMessage('✗ Insufficient disk space. Please free up space and try again.')
            } else {
                setMessage(`✗ Failed to install update: ${errorMsg}`)
            }
            
            setDownloading(false)
            setDownloadProgress(0)
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

    // Mark handlers as used to satisfy TypeScript while UI is commented out
    void handleToggleLlm
    void getLlmStatusBadge
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

            {/* Local LLM section - hidden until ready */}
            {/* <section className="settings-card">
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
            </section> */}

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

            {isTauriApp && (
                <section className="settings-card">
                    <div className="settings-card-header">
                        <span className="settings-icon">🔄</span>
                        <h3>Software Update</h3>
                        {appVersion && (
                            <span className="settings-model-name">v{appVersion}</span>
                        )}
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
            )}

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
