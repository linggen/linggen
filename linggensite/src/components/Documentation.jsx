function Documentation() {
    const docs = [
        {
            icon: '📖',
            title: 'Quickstart Guide',
            description: 'Install Linggen and index your first project in minutes.',
            url: '#get-started'
        },
        {
            icon: '⚙️',
            title: 'Configuration',
            description: 'Learn about sources, file patterns, and project profiles.',
            url: '#features'
        },
        {
            icon: '🗺️',
            title: 'Roadmap',
            description: "See what's planned for Linggen.",
            url: '#beta'
        }
    ]

    return (
        <section className="docs-section" id="docs">
            <div className="container">
                <h2 className="section-title">
                    <span className="title-decoration">◆</span>
                    Documentation
                    <span className="title-decoration">◆</span>
                </h2>
                <p className="section-description">
                    Get up to speed quickly with guides and references on GitHub.
                </p>

                <div className="docs-grid">
                    {docs.map((doc, index) => (
                        <a key={index} href={doc.url} target="_blank" rel="noreferrer" className="doc-card">
                            <div className="doc-icon">{doc.icon}</div>
                            <h3>{doc.title}</h3>
                            <p>{doc.description}</p>
                        </a>
                    ))}
                </div>
            </div>
        </section>
    )
}

export default Documentation
