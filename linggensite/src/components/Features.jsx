function Features() {
    const features = [
        {
            icon: '🧠',
            title: 'RAG-Powered Memory Layer',
            description: 'Linggen uses Retrieval-Augmented Generation (RAG) to give your AI tools deep context about your codebase and documents.',
            items: [
                'Semantic search across all your files',
                'Vector embeddings stored locally',
                'Automatic chunking & indexing',
                'Works offline once models downloaded'
            ]
        },
        {
            icon: '🔌',
            title: 'MCP Server Integration',
            description: 'Connect Linggen to AI coding tools like Cursor via the Model Context Protocol (MCP). One server, all your context.',
            items: [
                'Built-in MCP server at localhost:8787',
                'Works with Cursor IDE out of the box',
                'Team support: share one server across devs',
                'Extensible for other MCP-compatible tools'
            ]
        },
        {
            icon: '🔒',
            title: 'Privacy-First & Local',
            description: 'Your code stays on your machine. No cloud, no telemetry, no data leaving your device.',
            items: [
                'macOS desktop app (beta)',
                'High-performance Rust backend',
                'Local LanceDB vector store',
                'Windows & Linux coming soon'
            ]
        }
    ]

    return (
        <section className="features-section" id="features">
            <div className="container">
                <h2 className="section-title">
                    <span className="title-decoration">◆</span>
                    Core Features
                    <span className="title-decoration">◆</span>
                </h2>

                <div className="features-grid">
                    {features.map((feature, index) => (
                        <div key={index} className="feature-card">
                            <div className="feature-icon">{feature.icon}</div>
                            <h3>{feature.title}</h3>
                            <p>{feature.description}</p>
                            <ul className="feature-list">
                                {feature.items.map((item, i) => (
                                    <li key={i}>{item}</li>
                                ))}
                            </ul>
                        </div>
                    ))}
                </div>
            </div>
        </section>
    )
}

export default Features
