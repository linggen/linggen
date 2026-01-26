import { useState, useEffect, useRef } from 'react'
import { getAppSettings, updateAppSettings, clearAllData, type AppSettings } from '../api'

export function SettingsView() {
    const [settings, setSettings] = useState<AppSettings | null>(null)
    const [loading, setLoading] = useState(false)
    const [saving, setSaving] = useState(false)
    const [message, setMessage] = useState<string | null>(null)
    const [clearing, setClearing] = useState(false)
    const [showClearConfirm, setShowClearConfirm] = useState(false)
    const isMountedRef = useRef(true)

    useEffect(() => {
        isMountedRef.current = true
        const loadSettingsWithRetry = async (attempt = 1) => {
            try {
                if (!isMountedRef.current) return
                setLoading(true)
                const data = await getAppSettings()
                if (!isMountedRef.current) return
                setSettings(data)
                // LLM status check removed for simplified view
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

    const handleThemeChange = async (theme: 'dark' | 'light' | 'system') => {
        if (!settings || saving) return

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
            const data = await getAppSettings()
            setSettings(data)
            setMessage('✗ Failed to update theme')
        } finally {
            setSaving(false)
            setTimeout(() => setMessage(null), 3000)
        }
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

    // Mark loading as used if needed for spinners in future
    void loading;

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
                                                className={`px-6 py-2 rounded-xl text-[10px] font-black tracking-widest transition-all relative z-10 ${isSelected
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
                    <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl shadow-sm overflow-hidden transition-all hover:shadow-md">
                        <div className="flex items-center gap-3 px-6 py-4 bg-[var(--item-hover)]/30 border-b border-[var(--border-color)]">
                            <span className="text-lg">🔍</span>
                            <h3 className="text-sm font-bold text-[var(--text-active)]">Search Engine</h3>
                        </div>
                        <div className="p-8">
                            <div className="flex flex-col gap-8">
                                <div className="flex flex-col sm:flex-row justify-between sm:items-center gap-4">
                                    <div className="flex flex-col gap-1">
                                        <span className="text-sm font-bold text-[var(--text-primary)]">Embedding Model</span>
                                        <span className="text-xs text-[var(--text-muted)]">Local vector generation engine</span>
                                    </div>
                                    <div className="px-4 py-2 bg-[var(--bg-app)] rounded-xl border border-[var(--border-color)] shadow-inner">
                                        <span className="text-[11px] font-black text-[var(--accent)] font-mono tracking-wider">all-MiniLM-L6-v2</span>
                                    </div>
                                </div>
                                <div className="flex items-start gap-4 p-5 bg-green-500/5 border border-green-500/10 rounded-2xl transition-colors hover:bg-green-500/[0.08]">
                                    <div className="w-10 h-10 rounded-full bg-green-500/10 flex items-center justify-center text-green-500 text-lg flex-shrink-0">🛡️</div>
                                    <div className="flex flex-col gap-1 pt-0.5">
                                        <span className="text-xs font-black text-green-600 uppercase tracking-widest">Privacy First</span>
                                        <p className="text-xs text-green-600/80 leading-relaxed font-medium">All indexing and search operations are performed 100% locally. Your data never leaves this device.</p>
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
                    <section className="bg-[var(--bg-sidebar)] border border-[var(--border-color)] rounded-2xl shadow-sm overflow-hidden transition-all hover:shadow-md">
                        <div className="flex items-center gap-3 px-6 py-4 bg-[var(--item-hover)]/30 border-b border-[var(--border-color)]">
                            <span className="text-lg">🔄</span>
                            <h3 className="text-sm font-bold text-[var(--text-active)]">Software Update</h3>
                        </div>
                        <div className="p-8">
                            <div className="flex flex-col gap-4">
                                <p className="text-sm text-[var(--text-primary)]">
                                    To update Linggen to the latest version, run the following command in your terminal:
                                </p>
                                <code className="text-[12px] text-[var(--accent)] font-mono bg-black/5 dark:bg-white/5 p-4 rounded-xl border border-[var(--border-color)]/30">
                                    linggen update
                                </code>
                            </div>
                        </div>
                    </section>

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

