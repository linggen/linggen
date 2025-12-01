function Features() {
    const features = [
        {
            icon: '📂',
            title: 'Index Your Projects Locally',
            description: 'Point Linggen at your codebases and folders. It builds a private, searchable index on your machine – nothing leaves your device.',
            items: [
                'Local folders and project directories',
                'Source-aware indexing for code & docs',
                'Fast incremental re-indexing',
                'Respects .gitignore patterns'
            ]
        },
        {
            icon: '🔍',
            title: 'Semantic Search & Profiles',
            description: 'Ask natural-language questions and generate high-level profiles of your projects with AI assistance.',
            items: [
                'Semantic search across files and chunks',
                'Source stats: files, chunks, size',
                'AI-generated project profiles',
                'Works offline once models are downloaded'
            ]
        },
        {
            icon: '🔒',
            title: 'Privacy-First & Developer-Focused',
            description: 'Built for developers who care about privacy. Your code stays on your machine, and deeper integrations are on the roadmap.',
            items: [
                'macOS desktop app (beta)',
                'Local backend built in Rust',
                'Cursor / MCP integration (coming soon)',
                'VS Code extension & HTTP API (roadmap)'
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
