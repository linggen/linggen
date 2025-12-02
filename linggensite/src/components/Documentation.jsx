import { DOWNLOAD_URL } from '../constants'

function Documentation() {
    const docs = [
        {
            icon: '📖',
            title: 'Download',
            description: 'Get the latest Linggen release for macOS.',
            url: DOWNLOAD_URL,
            external: true
        },
        {
            icon: '⚙️',
            title: 'Features',
            description: 'Learn about sources, file patterns, and project profiles.',
            url: '#features',
            external: false
        },
        {
            icon: '🗺️',
            title: 'Roadmap',
            description: "See what's planned for Linggen.",
            url: '#beta',
            external: false
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
                        <a 
                            key={index} 
                            href={doc.url} 
                            target={doc.external ? "_blank" : "_self"} 
                            rel={doc.external ? "noopener noreferrer" : undefined} 
                            className="doc-card"
                        >
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
