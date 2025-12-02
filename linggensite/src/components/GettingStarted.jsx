import { useState } from 'react'
import { DOWNLOAD_URL } from '../constants'

function GettingStarted() {
    const [copiedIndex, setCopiedIndex] = useState(null)

    const steps = [
        {
            number: 1,
            title: 'Download Linggen for macOS',
            description: 'Get the latest beta release from GitHub:',
            code: null,
            downloadLink: DOWNLOAD_URL
        },
        {
            number: 2,
            title: 'Install the App',
            description: 'Install Linggen like any Mac application:',
            code: `1. Drag "Linggen" into Applications
2. On first launch, macOS may ask for confirmation
3. Grant file access when prompted`
        },
        {
            number: 3,
            title: 'Add a Source & Index',
            description: 'Start building your personal knowledge index:',
            code: `1. Open Linggen
2. Go to the "Sources" tab
3. Add a Local Folder (e.g. your main project)
4. Click "Index" and watch the progress`
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
                    Get Started
                    <span className="title-decoration">◆</span>
                </h2>

                <div className="steps-container">
                    {steps.map((step, index) => (
                        <div key={index} className="step">
                            <div className="step-number">{step.number}</div>
                            <div className="step-content">
                                <h3>{step.title}</h3>
                                <p>{step.description}</p>
                                {step.downloadLink ? (
                                    <div className="download-block">
                                        <a 
                                            href={step.downloadLink} 
                                            className="download-btn"
                                            target="_blank" 
                                            rel="noopener noreferrer"
                                        >
                                            ⬇️ Download from GitHub Releases
                                        </a>
                                        <p className="download-note">
                                            Download the <code>.dmg</code> file and open it to install.
                                        </p>
                                    </div>
                                ) : (
                                    <div className="code-block">
                                        <code>{step.code}</code>
                                        <button
                                            className="copy-btn"
                                            onClick={() => copyCode(step.code, index)}
                                        >
                                            {copiedIndex === index ? 'Copied!' : 'Copy'}
                                        </button>
                                    </div>
                                )}
                            </div>
                        </div>
                    ))}
                </div>

                <div className="next-steps">
                    <h3>What's Next?</h3>
                    <ul>
                        <li>Ask questions in the AI Assistant tab</li>
                        <li>Add more sources (projects, docs, notes)</li>
                        <li>Generate and customize project profiles</li>
                        <li>Send feedback or report issues to hello@linggen.dev</li>
                    </ul>
                </div>
            </div>
        </section>
    )
}

export default GettingStarted
