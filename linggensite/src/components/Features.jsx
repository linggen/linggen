function Features() {
    const features = [
        {
            icon: '📚',
            title: 'Universal Ingestion',
            description: 'Like absorbing spiritual energy, Linggen ingests knowledge from Git repositories, local files, documentation sites, and more.',
            items: [
                'Git repository indexing',
                'Local filesystem monitoring',
                'Web content crawling',
                'Real-time updates'
            ]
        },
        {
            icon: '🧠',
            title: 'Semantic Memory',
            description: 'Advanced vector embeddings create your personal knowledge core - enabling instant, context-aware search across all your information.',
            items: [
                'AI-powered embeddings',
                'Lightning-fast retrieval',
                'Contextual understanding',
                'Privacy-first local storage'
            ]
        },
        {
            icon: '⚡',
            title: 'Seamless Integration',
            description: 'Extend your powers into your daily workflow through integrations with VS Code, chat applications, and AI assistants.',
            items: [
                'MCP protocol support',
                'VS Code extension',
                'API endpoints',
                'Cross-platform compatible'
            ]
        }
    ]

    return (
        <section className="features-section" id="features">
            <div className="container">
                <h2 className="section-title">
                    <span className="title-decoration">◆</span>
                    Three Realms of Power
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
