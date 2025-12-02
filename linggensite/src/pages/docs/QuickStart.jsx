import { DOWNLOAD_URL } from '../../constants'

function QuickStart() {
    return (
        <article className="doc-article">
            <h1>🚀 Quick Start Guide</h1>
            <p className="doc-intro">
                Get up and running with Linggen in just a few minutes.
            </p>

            <section className="doc-section">
                <h2>1. Download & Install</h2>
                <p>
                    <a href={DOWNLOAD_URL} target="_blank" rel="noopener noreferrer">Download Linggen</a> (macOS Beta). 
                    Open the <code>.dmg</code> file and drag Linggen to your Applications folder.
                </p>
                <div className="doc-note">
                    <strong>Note:</strong> Currently macOS only. Windows & Linux coming soon.
                </div>
            </section>

            <section className="doc-section">
                <h2>2. First Launch</h2>
                <p>
                    On first launch, Linggen will download the embedding model (~100MB). 
                    This only happens once. The app runs a local backend at <code>localhost:8787</code>.
                </p>
                <p>
                    If macOS shows a security warning, go to <strong>System Settings → Privacy & Security</strong> and click "Open Anyway".
                </p>
            </section>

            <section className="doc-section">
                <h2>3. Add Your First Source</h2>
                <ol className="doc-steps">
                    <li>Go to the <strong>Sources</strong> tab</li>
                    <li>Click <strong>"Add Source"</strong></li>
                    <li>Choose <strong>"Local Folder"</strong></li>
                    <li>Select a project directory</li>
                    <li>Click <strong>"Index"</strong> to start building the vector index</li>
                </ol>
                <p>
                    Indexing time depends on the size of your project. A typical codebase (10k files) takes about 1-2 minutes.
                </p>
            </section>

            <section className="doc-section">
                <h2>4. Start Searching</h2>
                <p>
                    Once indexing completes, go to the <strong>AI Assistant</strong> tab. 
                    Ask questions about your codebase in natural language:
                </p>
                <ul className="doc-examples">
                    <li>"How does authentication work in this project?"</li>
                    <li>"Where is the database connection configured?"</li>
                    <li>"Show me examples of error handling"</li>
                </ul>
            </section>

            <section className="doc-section">
                <h2>Next Steps</h2>
                <ul>
                    <li>Add more sources (projects, documents, notes)</li>
                    <li>Set up <a href="/docs/mcp">MCP integration</a> with Cursor IDE</li>
                    <li>Learn about <a href="/docs/sources">file patterns</a> to control what gets indexed</li>
                </ul>
            </section>
        </article>
    )
}

export default QuickStart
