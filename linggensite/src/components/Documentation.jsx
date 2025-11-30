function Documentation() {
    const docs = [
        {
            icon: '📖',
            title: 'User Guide',
            description: 'Learn the fundamentals of knowledge cultivation',
            url: '#'
        },
        {
            icon: '⚙️',
            title: 'Configuration',
            description: 'Customize Linggen to match your workflow',
            url: '#'
        },
        {
            icon: '🔌',
            title: 'API Reference',
            description: 'Integrate Linggen into your tools',
            url: '#'
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
                    Deepen your understanding with comprehensive documentation and guides.
                </p>

                <div className="docs-grid">
                    {docs.map((doc, index) => (
                        <a key={index} href={doc.url} className="doc-card">
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
