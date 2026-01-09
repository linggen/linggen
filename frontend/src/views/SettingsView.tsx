/* eslint-disable @typescript-eslint/no-unused-vars */
import { useState, useEffect, useRef } from 'react'
import { getAppSettings, updateAppSettings, clearAllData, getAppStatus, retryInit, type AppSettings, type AppStatusResponse } from '../api'
import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { getVersion } from '@tauri-apps/api/app'
import { Command } from '@tauri-apps/plugin-shell'

type UpdaterDownloadEvent = {
    event: 'Started' | 'Progress' | 'Finished'
}

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
    const [restartReady, setRestartReady] = useState(false)
    const [restarting, setRestarting] = useState(false)

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
            .then((ver: string) => setAppVersion(ver))
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
        setRestartReady(false)
        setRestarting(false)
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
                    await update.downloadAndInstall((event: UpdaterDownloadEvent) => {
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
            
            // Installation finished. Ask the user to restart.
            setDownloadProgress(100)
            setDownloading(false)
            setRestartReady(true)
            setMessage('✓ Update installed! Restart to finish applying it.')
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

    const handleRestartNow = async () => {
        if (!isTauriApp) return
        setRestarting(true)
        setMessage('Restarting...')
        try {
            await relaunch()
        } catch (relaunchErr) {
            console.error('Failed to relaunch app:', relaunchErr)
            setMessage('✓ Update installed! Please restart the app manually. (Restart failed)')
            setRestarting(false)
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
        <div className="flex-1 overflow-y-auto p-6 flex flex-col gap-4 max-w-[700px]">
            {message && (
                <div className={`fixed top-20 right-10 p-3 rounded-md text-sm font-medium z-50 animate-in slide-in-from-right-5 ${message.startsWith('✓') ? 'bg-green-500/15 border border-green-500/30 text-green-400' : 'bg-red-500/15 border border-red-500/30 text-red-400'}`}>
                    {message}
                </div>
            )}

            <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-lg overflow-hidden">
                <div className="flex items-center gap-2.5 px-5 py-3.5 bg-black/20 border-b border-[var(--border-color)]">
                    <span className="text-base">💾</span>
                    <h3 className="m-0 text-sm font-semibold text-[var(--text-active)] border-none p-0">Data Storage</h3>
                </div>
                <div className="px-5 py-4">
                    <div className="flex justify-between items-center py-2.5 border-b border-white/5 last:border-b-0">
                        <span className="text-sm text-[var(--text-secondary)]">Search index</span>
                        <span className="font-mono text-[11px] text-[var(--text-secondary)] bg-black/30 px-2 py-1 rounded">~/Library/Application Support/Linggen/lancedb</span>
                    </div>
                    <div className="flex justify-between items-center py-2.5 border-b border-white/5 last:border-b-0">
                        <span className="text-sm text-[var(--text-secondary)]">Source metadata</span>
                        <span className="font-mono text-[11px] text-[var(--text-secondary)] bg-black/30 px-2 py-1 rounded">~/Library/Application Support/Linggen/metadata.redb</span>
                    </div>
                </div>
            </section>

            <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-lg overflow-hidden">
                <div className="flex items-center gap-2.5 px-5 py-3.5 bg-black/20 border-b border-[var(--border-color)]">
                    <span className="text-base">🔍</span>
                    <h3 className="m-0 text-sm font-semibold text-[var(--text-active)] border-none p-0">Search Engine</h3>
                </div>
                <div className="px-5 py-4">
                    <div className="flex justify-between items-center py-2.5 border-b border-white/5 last:border-b-0">
                        <span className="text-sm text-[var(--text-secondary)]">Embedding Model</span>
                        <span className="text-sm text-[var(--text-primary)] text-right max-w-[60%]">all-MiniLM-L6-v2</span>
                    </div>
                    <div className="flex justify-between items-center py-2.5 border-b border-white/5 last:border-b-0">
                        <span className="text-sm text-[var(--text-secondary)]">Privacy</span>
                        <span className="text-green-400 text-xs text-right max-w-[60%]">100% local · offline-capable · data never leaves your device</span>
                    </div>
                </div>
            </section>

            {/* Local LLM section - hidden until ready */}
            {/* <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-lg overflow-hidden">
                <div className="flex items-center gap-2.5 px-5 py-3.5 bg-black/20 border-b border-[var(--border-color)]">
                    <span className="text-base">🤖</span>
                    <h3 className="m-0 text-sm font-semibold text-[var(--text-active)] border-none p-0">Local LLM</h3>
                    <span className="ml-auto text-[11px] text-[var(--text-secondary)] bg-[var(--accent)]/15 px-2.5 py-0.5 rounded font-mono">Qwen3-4B</span>
                </div>
                <div className="px-5 py-4">
                    <div className="flex justify-between items-center py-2 border-b-0">
                        <div className="flex items-center gap-3">
                            <label className="relative inline-block w-11 h-6">
                                <input
                                    type="checkbox"
                                    className="sr-only peer"
                                    checked={!!settings?.llm_enabled}
                                    onChange={handleToggleLlm}
                                    disabled={loading || saving || llmInitializing || !settings}
                                />
                                <span className="absolute cursor-pointer inset-0 bg-[#404040] rounded-full transition-colors peer-checked:bg-[var(--accent)] after:content-[''] after:absolute after:h-[18px] after:w-[18px] after:left-[3px] after:bottom-[3px] after:bg-[#999] after:rounded-full after:transition-transform peer-checked:after:translate-x-[20px] peer-checked:after:bg-white peer-disabled:opacity-50 peer-disabled:cursor-not-allowed"></span>
                            </label>
                            <span className="text-sm text-[var(--text-secondary)]">Enable Local LLM</span>
                        </div>
                        {getLlmStatusBadge()}
                    </div>

                    {llmInitializing && llmProgress && (
                        <div className="flex items-center gap-2.5 px-3.5 py-3 mt-3 bg-blue-500/10 border border-blue-500/20 rounded-md text-xs text-blue-400">
                            <div className="w-3.5 h-3.5 border-2 border-blue-500/30 border-t-blue-400 rounded-full animate-spin"></div>
                            <span>{llmProgress}</span>
                        </div>
                    )}

                    {llmStatus === 'error' && !llmInitializing && llmProgress && (
                        <div className="px-3.5 py-2.5 mt-3 bg-red-500/10 border border-red-500/20 rounded-md text-xs text-red-400">
                            <span>⚠️ {llmProgress}</span>
                        </div>
                    )}

                    <p className="mt-3 text-xs text-[var(--text-secondary)] leading-relaxed">
                        Enables chat, profile generation, and AI-powered analysis. 
                        The model (~3GB) will be downloaded on first enable.
                    </p>
                </div>
            </section> */}

            <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-lg overflow-hidden">
                <div className="flex items-center gap-2.5 px-5 py-3.5 bg-black/20 border-b border-[var(--border-color)]">
                    <span className="text-base">📊</span>
                    <h3 className="m-0 text-sm font-semibold text-[var(--text-active)] border-none p-0">Analytics</h3>
                </div>
                <div className="px-5 py-4">
                    <div className="flex justify-between items-center py-2 border-b-0">
                        <div className="flex items-center gap-3">
                            <label className="relative inline-block w-11 h-6">
                                <input
                                    type="checkbox"
                                    className="sr-only peer"
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
                                <span className="absolute cursor-pointer inset-0 bg-[#404040] rounded-full transition-colors peer-checked:bg-[var(--accent)] after:content-[''] after:absolute after:h-[18px] after:w-[18px] after:left-[3px] after:bottom-[3px] after:bg-[#999] after:rounded-full after:transition-transform peer-checked:after:translate-x-[20px] peer-checked:after:bg-white peer-disabled:opacity-50 peer-disabled:cursor-not-allowed"></span>
                            </label>
                            <span className="text-sm text-[var(--text-secondary)]">Help improve Linggen</span>
                        </div>
                        <span className={`text-[11px] px-2.5 py-1 rounded font-medium ${settings?.analytics_enabled !== false ? 'bg-green-500/15 text-green-400' : 'bg-neutral-500/20 text-neutral-500'}`}>
                            {settings?.analytics_enabled !== false ? 'Enabled' : 'Disabled'}
                        </span>
                    </div>
                    <p className="mt-3 text-xs text-[var(--text-secondary)] leading-relaxed">
                        Send anonymous usage statistics to help improve Linggen. 
                        We only collect basic usage data (app launches, sources added) — <strong className="text-[var(--text-primary)]">no code content, file paths, or personal information is ever sent</strong>.
                    </p>
                </div>
            </section>

            {isTauriApp && (
                <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-lg overflow-hidden">
                    <div className="flex items-center gap-2.5 px-5 py-3.5 bg-black/20 border-b border-[var(--border-color)]">
                        <span className="text-base">🔄</span>
                        <h3 className="m-0 text-sm font-semibold text-[var(--text-active)] border-none p-0">Software Update</h3>
                        {appVersion && (
                            <span className="ml-auto text-[11px] text-[var(--text-secondary)] bg-[var(--accent)]/15 px-2.5 py-0.5 rounded font-mono">v{appVersion}</span>
                        )}
                    </div>
                    <div className="px-5 py-4">
                        <div className="flex justify-between items-center py-2.5 border-b border-white/5 last:border-b-0">
                            <div className="flex-1">
                                <strong className="block text-sm text-[var(--text-primary)] mb-1">Application Updates</strong>
                                <p className="m-0 text-xs text-[var(--text-secondary)] leading-tight">
                                    Check for new versions and install updates automatically.
                                </p>
                            </div>
                            <button
                                type="button"
                                className="btn-secondary whitespace-nowrap"
                                onClick={handleCheckForUpdates}
                                disabled={checkingUpdate || downloading}
                            >
                                {checkingUpdate ? 'Checking...' : 'Check for Updates'}
                            </button>
                        </div>

                        {updateAvailable && updateInfo && (
                            <div className="mt-3 px-3.5 py-3 bg-blue-500/10 border border-blue-500/20 rounded-md">
                                <div className="flex justify-between items-center mb-2">
                                    <div>
                                        <strong className="text-blue-400 text-sm">
                                            Version {updateInfo.version} Available
                                        </strong>
                                        {updateInfo.date && (
                                            <div className="text-[11px] text-[var(--text-secondary)] mt-0.5">
                                                Released: {new Date(updateInfo.date).toLocaleDateString()}
                                            </div>
                                        )}
                                    </div>
                                    <button
                                        type="button"
                                        className="bg-blue-400 border border-blue-400 text-white px-3 py-1.5 rounded !text-[11px] font-semibold cursor-pointer uppercase tracking-wider hover:bg-blue-500 transition-all"
                                        onClick={handleInstallUpdate}
                                        disabled={downloading || restarting}
                                    >
                                        {downloading ? 'Installing...' : 'Install Update'}
                                    </button>
                                </div>
                                {updateInfo.body && (
                                    <div className="text-xs text-[var(--text-secondary)] mt-2 pt-2 border-t border-blue-500/20 max-h-[150px] overflow-y-auto">
                                        <strong className="text-[var(--text-primary)]">What's New:</strong>
                                        <pre className="whitespace-pre-wrap font-inherit mt-1">{updateInfo.body}</pre>
                                    </div>
                                )}
                                {downloading && downloadProgress > 0 && (
                                    <div className="mt-2">
                                        <div className="w-full h-1 bg-blue-500/20 rounded overflow-hidden">
                                            <div className="h-full bg-blue-400 transition-[width] duration-300 ease-in-out" style={{ width: `${downloadProgress}%` }} />
                                        </div>
                                        <div className="text-[11px] text-[var(--text-secondary)] mt-1">
                                            Downloading... {downloadProgress}%
                                        </div>
                                    </div>
                                )}

                                {restartReady && (
                                    <div className="mt-2.5 flex justify-end">
                                        <button
                                            type="button"
                                            className="bg-green-500 border border-green-500 text-white px-3 py-1.5 rounded !text-[11px] font-semibold cursor-pointer uppercase tracking-wider hover:bg-green-600 transition-all"
                                            onClick={handleRestartNow}
                                            disabled={restarting}
                                        >
                                            {restarting ? 'Restarting...' : 'Restart Now'}
                                        </button>
                                    </div>
                                )}
                            </div>
                        )}

                        <p className="mt-3 text-xs text-[var(--text-secondary)] leading-relaxed">
                            Updates are downloaded from the official Linggen releases repository. 
                            After installation, click “Restart Now” to finish applying the update.
                        </p>
                    </div>
                </section>
            )}

            <section className="bg-[var(--bg-sidebar)] border border-red-500/30 rounded-lg overflow-hidden">
                <div className="flex items-center gap-2.5 px-5 py-3.5 bg-red-500/10 border-b border-red-500/20">
                    <span className="text-base">⚠️</span>
                    <h3 className="m-0 text-sm font-semibold text-red-400 border-none p-0">Danger Zone</h3>
                </div>
                <div className="px-5 py-4">
                    <div className="flex justify-between items-center gap-5">
                        <div className="flex-1">
                            <strong className="block text-sm text-[var(--text-primary)] mb-1">Clear All Data</strong>
                            <p className="m-0 text-xs text-[var(--text-secondary)] leading-tight">Permanently delete all indexed data, sources, profiles, and settings. This cannot be undone.</p>
                        </div>
                        <button
                            type="button"
                            className="btn-danger whitespace-nowrap"
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
                <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-[9999]" onClick={() => setShowClearConfirm(false)}>
                    <div className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-xl w-[420px] max-w-[90vw] shadow-2xl" onClick={e => e.stopPropagation()}>
                        <div className="px-5 py-4 border-b border-[var(--border-color)]">
                            <h3 className="m-0 text-sm font-semibold text-[var(--text-active)]">⚠️ Clear All Data</h3>
                        </div>
                        <div className="p-5">
                            <p className="mb-4 text-[var(--text-primary)]">
                                This will <strong>permanently delete</strong>:
                            </p>
                            <ul className="m-0 mb-4 ml-6 text-[var(--text-secondary)] leading-[1.8] list-disc text-sm">
                                <li>All indexed chunks in vector database</li>
                                <li>All source configurations</li>
                                <li>All project profiles</li>
                                <li>All indexing history</li>
                                <li>All uploaded files</li>
                            </ul>
                            <p className="mb-2 text-[var(--text-secondary)] text-[0.85rem]">
                                ✓ Your settings and downloaded models will be preserved.
                            </p>
                            <p className="text-red-500 font-semibold">
                                This action cannot be undone!
                            </p>
                        </div>
                        <div className="flex justify-end gap-2.5 px-5 py-4 border-t border-[var(--border-color)] bg-black/10">
                            <button
                                type="button"
                                className="btn-secondary"
                                onClick={() => setShowClearConfirm(false)}
                            >
                                Cancel
                            </button>
                            <button
                                type="button"
                                className="btn-danger whitespace-nowrap"
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
