import { NavLink, Outlet } from 'react-router-dom'

function DocsLayout() {
    const navItems = [
        { path: '/docs', label: '🚀 Quick Start', end: true },
        { path: '/docs/sources', label: '📂 Adding Sources' },
        { path: '/docs/mcp', label: '🔌 MCP Setup' },
        { path: '/docs/search', label: '🔍 Search & Chat' },
    ]

    return (
        <div className="docs-page">
            <nav className="docs-nav">
                <div className="docs-nav-header">
                    <NavLink to="/" className="docs-back-link">← Back to Home</NavLink>
                    <h2>Documentation</h2>
                </div>
                <ul className="docs-nav-list">
                    {navItems.map(item => (
                        <li key={item.path}>
                            <NavLink 
                                to={item.path} 
                                end={item.end}
                                className={({ isActive }) => isActive ? 'active' : ''}
                            >
                                {item.label}
                            </NavLink>
                        </li>
                    ))}
                </ul>
            </nav>
            <main className="docs-content">
                <Outlet />
            </main>
        </div>
    )
}

export default DocsLayout
