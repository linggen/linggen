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

    const isTauriApp =
        typeof window !== 'undefined' &&
        (!!(window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ ||
            !!(window as unknown as { __TAURI__?: unknown }).__TAURI__)

    const [appVersion, setAppVersion] = useState<string | null>(null)
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
                const data = await getAppSettings()
                if (!isMountedRef.current) return
                setSettings(data)
                if (data.llm_enabled) {
                    const status = await getAppStatus()
                    if (isMountedRef.current) updateLlmStatusFromAppStatus(status)
                }
            } catch (err) {
                console.error(err)
                if (attempt < 3 && isMountedRef.current) setTimeout(() => loadSettingsWithRetry(attempt + 1), 1000)
            } finally {
                if (isMountedRef.current) setLoading(false)
            }
        }
        loadSettingsWithRetry()
        return () => { isMountedRef.current = false }
    }, [])

    useEffect(() => {
        if (!isTauriApp) return
        getVersion().then(setAppVersion).catch(console.error)
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

    const handleThemeChange = async (theme: 'dark' | 'light' | 'system') => {
        if (!settings || saving) return
        
        // Use functional update for robustness and immediate local feedback
        setSettings(prev => prev ? { ...prev, theme } : null)
        setSaving(true)
        
        try {
            await updateAppSettings({ ...settings, theme })
            const root = document.documentElement
            if (theme === 'system') root.removeAttribute('data-theme')
            else root.setAttribute('data-theme', theme)
            setMessage('✓ Theme updated')
        } catch (err) {
            console.error('Failed to update theme:', err)
            // Rollback on failure
            const data = await getAppSettings()
            setSettings(data)
            setMessage('✗ Failed to update theme')
        } finally {
            setSaving(false)
            setTimeout(() => setMessage(null), 3000)
        }
    }

    const handleCheckForUpdates = async () => {
        setCheckingUpdate(true)
        try {
            const update = await check()
            if (update) {
                setUpdateAvailable(true)
                setUpdateInfo({ version: update.version, date: update.date, body: update.body })
                setMessage(`✓ Update available: v${update.version}`)
            } else {
                setMessage('✓ You are up to date')
            }
        } catch (err) {
            console.error(err)
            setMessage('✗ Update check failed')
        } finally {
            setCheckingUpdate(false)
            setTimeout(() => setMessage(null), 5000)
        }
    }

    const handleInstallUpdate = async () => {
        setDownloading(true)
        try {
            const update = await check()
            if (update) {
                await update.downloadAndInstall((event: UpdaterDownloadEvent) => {
                    if (event.event === 'Progress') setDownloadProgress((prev) => Math.min(prev + 5, 95))
                    else if (event.event === 'Finished') setDownloadProgress(100)
                })
                setRestartReady(true)
                setMessage('✓ Update installed')
            }
        } catch (err) {
            console.error(err)
            setMessage('✗ Installation failed')
        } finally {
            setDownloading(false)
        }
    }

    const handleRestartNow = async () => {
        setRestarting(true)
        try { await relaunch() } catch { setRestarting(false) }
    }

    const handleClearAllData = () => setShowClearConfirm(true)

    const confirmClearAllData = async () => {
        setShowClearConfirm(false)
        setClearing(true)
        try {
            await clearAllData()
            setMessage('✓ Data cleared. Refreshing...')
            setTimeout(() => window.location.reload(), 1500)
        } catch (err) {
            console.error(err)
            setMessage('✗ Failed to clear data')
        } finally {
            setClearing(false)
        }
    }

    const getLlmStatusBadge = () => {
        if (!settings?.llm_enabled) return <span className="text-[10px] bg-gray-500/20 text-gray-500 px-2 py-0.5 rounded">Disabled</span>
        if (llmStatus === 'ready') return <span className="text-[10px] bg-green-500/20 text-green-500 px-2 py-0.5 rounded">Ready</span>
        return <span className="text-[10px] bg-blue-500/20 text-blue-500 px-2 py-0.5 rounded">Active</span>
    }

    // Mark unused to satisfy linter
    void loading; void llmInitializing; void llmProgress; void getLlmStatusBadge;

    return (
        <div className="flex-1 overflow-y-auto w-full bg-[var(--bg-app)]">
            <div className="max-w-[800px] mx-auto p-8 flex flex-col gap-10 pb-32">
                {message && (
                    <div className={`fixed top-6 right-6 p-4 rounded-xl text-xs font-bold z-[100] shadow-2xl animate-in slide-in-from-top-4 border ${message.startsWith('✓') ? 'bg-green-500 text-white border-green-400' : 'bg-red-500 text-white border-red-400'}`}>
                        {message}
                    </div>
                )}

                <header className="flex flex-col gap-2">
                    <h2 className="text-3xl font-extrabold text-[var(--text-active)] tracking-tight">Settings</h2>
                    <p className="text-sm text-[var(--text-secondary)] opacity-80 font-medium">Configure your experience and manage application data.</p>
                </header>

                <div className="flex flex-col gap-8">
                    {/* Appearance */}
                    <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl shadow-sm overflow-hidden">
                        <div className="flex items-center gap-3 px-6 py-4 bg-[var(--item-hover)]/30 border-b border-[var(--border-color)]">
                            <span className="text-lg">🎨</span>
                            <h3 className="text-sm font-bold text-[var(--text-active)]">Appearance</h3>
                        </div>
                        <div className="p-8">
                            <div className="flex flex-col sm:flex-row justify-between sm:items-center gap-6">
                                <div className="flex flex-col gap-1">
                                    <span className="text-sm font-bold text-[var(--text-primary)]">Interface Theme</span>
                                    <span className="text-xs text-[var(--text-muted)]">Select how Linggen looks in your system</span>
                                </div>
                                <div className="flex bg-[var(--bg-app)] border border-[var(--border-color)] rounded-2xl p-1 shadow-inner relative">
                                    {(['system', 'light', 'dark'] as const).map((mode) => {
                                        const isSelected = (settings?.theme || 'system') === mode;
                                        return (
                                            <button
                                                key={mode}
                                                onClick={() => handleThemeChange(mode)}
                                                className={`px-6 py-2 rounded-xl text-[10px] font-black tracking-widest transition-all relative z-10 ${
                                                    isSelected
                                                        ? 'text-white'
                                                        : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                                                }`}
                                            >
                                                {isSelected && (
                                                    <div className="absolute inset-0 bg-[var(--accent)] rounded-xl shadow-[0_2px_8px_rgba(113,55,241,0.4)] border border-[#c084fc]/50 z-[-1] animate-in fade-in zoom-in-95 duration-200"></div>
                                                )}
                                                {mode.toUpperCase()}
                                            </button>
                                        );
                                    })}
                                </div>
                            </div>
                        </div>
                    </section>

                    {/* Data Storage */}
                    <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl shadow-sm overflow-hidden">
                        <div className="flex items-center gap-3 px-6 py-4 bg-[var(--item-hover)]/30 border-b border-[var(--border-color)]">
                            <span className="text-lg">💾</span>
                            <h3 className="text-sm font-bold text-[var(--text-active)]">Data Storage</h3>
                        </div>
                        <div className="p-8 flex flex-col gap-6">
                            <div className="grid grid-cols-1 gap-5">
                                <div className="flex flex-col gap-3 p-6 bg-[var(--bg-app)] rounded-2xl border-2 border-[var(--border-color)] shadow-sm group hover:border-[var(--accent)] transition-colors">
                                    <div className="flex items-center gap-2">
                                        <span className="text-[10px] font-black text-[var(--text-active)] uppercase tracking-[0.2em]">Search Index</span>
                                        <span className="h-px flex-1 bg-[var(--border-color)] opacity-50"></span>
                                    </div>
                                    <code className="text-[12px] text-[var(--text-primary)] font-mono leading-relaxed break-all bg-black/5 dark:bg-white/5 p-3 rounded-lg border border-[var(--border-color)]/30">~/Library/Application Support/Linggen/lancedb</code>
                                </div>
                                <div className="flex flex-col gap-3 p-6 bg-[var(--bg-app)] rounded-2xl border-2 border-[var(--border-color)] shadow-sm group hover:border-[var(--accent)] transition-colors">
                                    <div className="flex items-center gap-2">
                                        <span className="text-[10px] font-black text-[var(--text-active)] uppercase tracking-[0.2em]">Metadata Database</span>
                                        <span className="h-px flex-1 bg-[var(--border-color)] opacity-50"></span>
                                    </div>
                                    <code className="text-[12px] text-[var(--text-primary)] font-mono leading-relaxed break-all bg-black/5 dark:bg-white/5 p-3 rounded-lg border border-[var(--border-color)]/30">~/Library/Application Support/Linggen/metadata.redb</code>
                                </div>
                            </div>
                        </div>
                    </section>

                    {/* Search Engine */}
                    <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl shadow-sm overflow-hidden">
                        <div className="flex items-center gap-3 px-6 py-4 bg-[var(--item-hover)]/30 border-b border-[var(--border-color)]">
                            <span className="text-lg">🔍</span>
                            <h3 className="text-sm font-bold text-[var(--text-active)]">Search Engine</h3>
                        </div>
                        <div className="p-8">
                            <div className="flex flex-col gap-6">
                                <div className="flex justify-between items-center pb-4 border-b border-[var(--border-color)]/40">
                                    <div className="flex flex-col gap-1">
                                        <span className="text-sm font-bold text-[var(--text-primary)]">Embedding Model</span>
                                        <span className="text-xs text-[var(--text-muted)]">Local vector generation model</span>
                                    </div>
                                    <span className="px-3 py-1.5 bg-[var(--bg-app)] rounded-lg border border-[var(--border-color)] text-[11px] font-bold text-[var(--text-primary)] font-mono">all-MiniLM-L6-v2</span>
                                </div>
                                <div className="flex items-center gap-4 p-4 bg-green-500/5 border border-green-500/10 rounded-xl">
                                    <div className="w-8 h-8 rounded-full bg-green-500/10 flex items-center justify-center text-green-500">🛡️</div>
                                    <div className="flex flex-col">
                                        <span className="text-[11px] font-bold text-green-600">Privacy Guaranteed</span>
                                        <span className="text-[10px] text-green-600/70">All indexing and search happens 100% locally on your machine.</span>
                                    </div>
                                </div>
                            </div>
                        </div>
                    </section>

                    {/* Analytics */}
                    <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl shadow-sm overflow-hidden">
                        <div className="flex items-center gap-3 px-6 py-4 bg-[var(--item-hover)]/30 border-b border-[var(--border-color)]">
                            <span className="text-lg">📊</span>
                            <h3 className="text-sm font-bold text-[var(--text-active)]">Analytics</h3>
                        </div>
                        <div className="p-8">
                            <div className="flex justify-between items-start gap-8">
                                <div className="flex flex-col gap-1">
                                    <span className="text-sm font-bold text-[var(--text-primary)]">Usage Statistics</span>
                                    <p className="text-xs text-[var(--text-muted)] leading-relaxed">Share anonymous data to help us improve. No code content or personal info is ever tracked.</p>
                                </div>
                                <label className="relative inline-block w-12 h-6 flex-shrink-0 cursor-pointer group">
                                    <input
                                        type="checkbox"
                                        className="sr-only peer"
                                        checked={settings?.analytics_enabled ?? true}
                                        onChange={async () => {
                                            if (!settings || saving) return
                                            const next = { ...settings, analytics_enabled: !settings.analytics_enabled }
                                            setSettings(next)
                                            setSaving(true)
                                            try { await updateAppSettings(next); setMessage('✓ Saved') }
                                            catch { setMessage('✗ Failed') }
                                            finally { setSaving(false); setTimeout(() => setMessage(null), 3000) }
                                        }}
                                        disabled={saving}
                                    />
                                    <div className="w-full h-full bg-[var(--bg-app)] border-2 border-[var(--border-color)] rounded-full transition-all duration-300 peer-checked:bg-[var(--accent)] peer-checked:border-[var(--accent)] group-hover:border-[var(--accent)]/50"></div>
                                    <div className="absolute top-1 left-1 w-4 h-4 bg-[var(--text-muted)] rounded-full transition-all duration-300 peer-checked:translate-x-6 peer-checked:bg-white peer-checked:shadow-md shadow-sm"></div>
                                </label>
                            </div>
                        </div>
                    </section>

                    {/* Software Update */}
                    {isTauriApp && (
                        <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl shadow-sm overflow-hidden">
                            <div className="flex items-center gap-3 px-6 py-4 bg-[var(--item-hover)]/30 border-b border-[var(--border-color)]">
                                <span className="text-lg">🔄</span>
                                <h3 className="text-sm font-bold text-[var(--text-active)]">Software Update</h3>
                                {appVersion && <span className="ml-auto px-2.5 py-1 bg-[var(--bg-app)] text-[10px] font-black text-[var(--text-muted)] rounded-full border border-[var(--border-color)] shadow-sm">v{appVersion}</span>}
                            </div>
                            <div className="p-8">
                                <div className="flex flex-col gap-6">
                                    <div className="flex justify-between items-center">
                                        <div className="flex flex-col gap-1">
                                            <span className="text-sm font-bold text-[var(--text-primary)]">Application Updates</span>
                                            <span className="text-xs text-[var(--text-muted)]">Check for new versions and improvements</span>
                                        </div>
                                        <button
                                            className="btn-secondary !py-2 !px-6 !text-[10px] !font-black !rounded-xl"
                                            onClick={handleCheckForUpdates}
                                            disabled={checkingUpdate || downloading}
                                        >
                                            {checkingUpdate ? 'CHECKING...' : 'CHECK NOW'}
                                        </button>
                                    </div>

                                    {updateAvailable && updateInfo && (
                                        <div className="p-6 bg-blue-500/5 border border-blue-500/10 rounded-2xl animate-in zoom-in-95 duration-300">
                                            <div className="flex justify-between items-start mb-6">
                                                <div className="flex flex-col gap-1">
                                                    <div className="flex items-center gap-2">
                                                        <span className="w-2.5 h-2.5 rounded-full bg-blue-500 animate-pulse"></span>
                                                        <span className="text-sm font-black text-blue-500">NEW VERSION READY</span>
                                                    </div>
                                                    <span className="text-[10px] font-bold text-[var(--text-muted)]">v{updateInfo.version} • {updateInfo.date ? new Date(updateInfo.date).toLocaleDateString() : 'LATEST'}</span>
                                                </div>
                                                {!restartReady && (
                                                    <button className="btn-primary !py-2 !px-6 !text-[10px] !font-black !rounded-xl" onClick={handleInstallUpdate} disabled={downloading}>
                                                        {downloading ? 'DOWNLOADING...' : 'INSTALL NOW'}
                                                    </button>
                                                )}
                                            </div>

                                            {downloading && (
                                                <div className="w-full h-1.5 bg-[var(--bg-app)] rounded-full overflow-hidden mb-4">
                                                    <div className="h-full bg-blue-500 transition-all duration-500" style={{ width: `${downloadProgress}%` }} />
                                                </div>
                                            )}

                                            {restartReady && (
                                                <div className="flex justify-end">
                                                    <button className="bg-green-500 hover:bg-green-600 text-white px-8 py-3 rounded-xl text-[10px] font-black tracking-widest transition-all shadow-lg hover:scale-105 active:scale-95" onClick={handleRestartNow}>
                                                        RESTART TO APPLY
                                                    </button>
                                                </div>
                                            )}
                                        </div>
                                    )}
                                </div>
                            </div>
                        </section>
                    )}

                    {/* Danger Zone */}
                    <section className="border border-red-500/20 rounded-2xl shadow-sm overflow-hidden bg-red-500/[0.02]">
                        <div className="flex items-center gap-3 px-6 py-4 bg-red-500/5 border-b border-red-500/10">
                            <span className="text-lg">⚠️</span>
                            <h3 className="text-sm font-bold text-red-500">Danger Zone</h3>
                        </div>
                        <div className="p-8">
                            <div className="flex flex-col sm:flex-row justify-between sm:items-center gap-8">
                                <div className="flex flex-col gap-1">
                                    <span className="text-sm font-bold text-red-500">Clear All Local Data</span>
                                    <p className="text-xs text-[var(--text-muted)]">This will wipe all vector indices, sources, and settings. Impossible to undo.</p>
                                </div>
                                <button
                                    className="btn-danger !py-2.5 !px-8 !text-[10px] !font-black !rounded-xl shadow-lg shadow-red-500/10 hover:bg-red-500 hover:text-white transition-all"
                                    onClick={handleClearAllData}
                                    disabled={clearing}
                                >
                                    {clearing ? 'WIPING DATA...' : 'WIPE EVERYTHING'}
                                </button>
                            </div>
                        </div>
                    </section>
                </div>
            </div>

            {showClearConfirm && (
                <div className="fixed inset-0 bg-black/70 backdrop-blur-sm flex items-center justify-center z-[9999] p-6" onClick={() => setShowClearConfirm(false)}>
                    <div className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-3xl w-[450px] max-w-full shadow-2xl animate-in zoom-in-95 duration-200" onClick={e => e.stopPropagation()}>
                        <div className="p-8 border-b border-[var(--border-color)]">
                            <h3 className="text-xl font-black text-[var(--text-active)] flex items-center gap-3">
                                <span className="text-2xl">🚨</span> Final Warning
                            </h3>
                        </div>
                        <div className="p-8">
                            <p className="text-[var(--text-primary)] text-sm font-bold mb-4 leading-relaxed">
                                You are about to permanently delete all indexed data and configurations.
                            </p>
                            <div className="p-5 bg-red-500/5 border border-red-500/10 rounded-2xl">
                                <p className="text-red-500 text-xs font-black uppercase tracking-widest mb-1">Irreversible Action</p>
                                <p className="text-red-600/70 text-xs font-medium">All vector embeddings, metadata, and local indices will be lost forever.</p>
                            </div>
                        </div>
                        <div className="flex justify-end gap-3 p-8 pt-4">
                            <button className="btn-secondary !rounded-xl !px-6" onClick={() => setShowClearConfirm(false)}>CANCEL</button>
                            <button className="btn-danger !rounded-xl !px-6 !bg-red-500 !text-white font-black" onClick={confirmClearAllData}>DELETE ALL DATA</button>
                        </div>
                    </div>
                </div>
            )}
        </div>
    )
}
