import { useState } from 'react'

function GettingStarted() {
    const [copiedIndex, setCopiedIndex] = useState(null)

    const steps = [
        {
            number: 1,
            title: 'Download Linggen',
            description: 'Get the installer for your platform:',
            code: `# macOS
Download: linggen-mac.dmg

# Linux (Debian/Ubuntu)
Download: linggen-linux.deb

# Linux (Fedora/RHEL)
Download: linggen-linux.rpm`
        },
        {
            number: 2,
            title: 'Install Linggen',
            description: 'Install using your platform\'s package manager:',
            code: `# macOS: Open DMG and drag to Applications

# Linux (Debian/Ubuntu)
sudo dpkg -i linggen-linux.deb

# Linux (Fedora/RHEL)
sudo rpm -i linggen-linux.rpm`
        },
        {
            number: 3,
            title: 'Launch & Index',
            description: 'Start Linggen and add your first knowledge source:',
            code: `# Launch the application
linggen

# Index a Git repository
linggen index --type git --path /path/to/repo

# Index a local folder
linggen index --type folder --path /path/to/docs`
        },
        {
            number: 4,
            title: 'Search & Explore',
            description: 'Query your knowledge base with semantic search:',
            code: `# Search your indexed content
linggen search "your query here"

# Or use the web interface
open http://localhost:8080`
        }
    ]

    const copyCode = (code, index) => {
        navigator.clipboard.writeText(code)
        setCopiedIndex(index)
        setTimeout(() => setCopiedIndex(null), 2000)
    }

    return (
        <section className="getting-started-section" id="get-started">
            <div className="container">
                <h2 className="section-title">
                    <span className="title-decoration">◆</span>
                    Begin Your Journey
                    <span className="title-decoration">◆</span>
                </h2>

                <div className="steps-container">
                    {steps.map((step, index) => (
                        <div key={index} className="step">
                            <div className="step-number">{step.number}</div>
                            <div className="step-content">
                                <h3>{step.title}</h3>
                                <p>{step.description}</p>
                                <div className="code-block">
                                    <code>{step.code}</code>
                                    <button
                                        className="copy-btn"
                                        onClick={() => copyCode(step.code, index)}
                                    >
                                        {copiedIndex === index ? 'Copied!' : 'Copy'}
                                    </button>
                                </div>
                            </div>
                        </div>
                    ))}
                </div>

                <div className="next-steps">
                    <h3>Next Steps</h3>
                    <ul>
                        <li>Download the latest release from our website</li>
                        <li>Index additional sources (Git repos, websites, documents)</li>
                        <li>Explore the web interface and API</li>
                        <li>Set up VS Code integration for seamless workflow</li>
                        <li>Join the community and contribute</li>
                    </ul>
                </div>
            </div>
        </section>
    )
}

export default GettingStarted
