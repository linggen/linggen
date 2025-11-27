import { useState, useEffect } from 'react'
import { getAppSettings, updateAppSettings, clearAllData, type AppSettings } from '../api'

export function SettingsView() {
    const [settings, setSettings] = useState<AppSettings | null>(null)
    const [loading, setLoading] = useState(false)
    const [saving, setSaving] = useState(false)
    const [message, setMessage] = useState<string | null>(null)
    const [clearing, setClearing] = useState(false)

    useEffect(() => {
        const loadSettings = async () => {
            try {
                setLoading(true)
                const data = await getAppSettings()
                setSettings(data)
            } catch (err) {
                console.error('Failed to load app settings:', err)
                setMessage('✗ Failed to load settings')
            } finally {
                setLoading(false)
            }
        }
        loadSettings()
    }, [])

    const handleToggleIntent = async () => {
        if (!settings || saving) return
        const next = { ...settings, intent_detection_enabled: !settings.intent_detection_enabled }
        setSettings(next)
        setSaving(true)
        setMessage(null)
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
                    <div className="settings-item">
                        <span className="settings-label">LLM Model</span>
                        <span className="settings-value">Qwen3-4B-Instruct (local)</span>
                    </div>
                </div>

                <div className="settings-group">
                    <h3>AI Pipeline</h3>
                    {message && (
                        <div className={`status ${message.startsWith('✓') ? 'success' : 'error'}`} style={{ marginBottom: '0.75rem' }}>
                            {message}
                        </div>
                    )}
                    <div className="settings-item">
                        <span className="settings-label">LLM Intent Detection</span>
                        <span className="settings-value">
                            <label style={{ display: 'inline-flex', alignItems: 'center', gap: '0.5rem', cursor: 'pointer' }}>
                                <input
                                    type="checkbox"
                                    checked={!!settings?.intent_detection_enabled}
                                    onChange={handleToggleIntent}
                                    disabled={loading || saving || !settings}
                                />
                                <span>{settings?.intent_detection_enabled ? 'Enabled' : 'Disabled'}</span>
                            </label>
                        </span>
                    </div>
                    <div className="settings-item settings-item-muted">
                        <span>
                            When disabled, the enhancer skips the LLM-based intent detector and treats all queries as general questions.
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
