interface AppHeaderProps {
    status: 'idle' | 'initializing' | 'indexing' | 'error'
    message?: string | null
    onRetry?: () => void
}

import logo from '../assets/logo.svg'

export function AppHeader({ status, message, onRetry }: AppHeaderProps) {
    let statusText = 'Idle'
    let statusClassName = 'status-pill idle'

    if (status === 'initializing') {
        statusText = message || 'Initializing'
        statusClassName = 'status-pill initializing'
    } else if (status === 'indexing') {
        statusText = 'Indexing'
        statusClassName = 'status-pill indexing'
    } else if (status === 'error') {
        statusText = 'Error'
        statusClassName = 'status-pill error'
    }

    return (
        <header className="app-header">
            <div>
                <div style={{ display: 'flex', alignItems: 'center', gap: '0.75rem', marginBottom: '0.5rem' }}>
                    <img src={logo} alt="Linggen Logo" style={{ width: '32px', height: '32px' }} />
                    <h1 style={{ margin: 0, fontSize: '1.5rem' }}>Linggen</h1>
                </div>
                <p>Your personal knowledge hub. Search everything, instantly.</p>
            </div>
            <div style={{ display: 'flex', alignItems: 'center', gap: '1rem' }}>
                <div className={statusClassName}>
                    <span className="status-dot" />
                    <span>{statusText}</span>
                </div>
                {status === 'error' && onRetry && (
                    <button
                        onClick={onRetry}
                        style={{
                            padding: '0.5rem 1rem',
                            background: 'var(--primary)',
                            color: 'white',
                            border: 'none',
                            borderRadius: '6px',
                            cursor: 'pointer',
                            fontSize: '0.9rem',
                            fontWeight: '500',
                        }}
                    >
                        Retry
                    </button>
                )}
            </div>
        </header>
    )
}
