function Sources() {
    return (
        <article className="doc-article">
            <h1>📂 Adding & Managing Sources</h1>
            <p className="doc-intro">
                Sources are the knowledge bases that Linggen indexes and searches.
            </p>

            <section className="doc-section">
                <h2>Source Types</h2>
                <p>Linggen supports multiple source types:</p>
                
                <div className="doc-grid">
                    <div className="doc-card-small">
                        <h3>📁 Local Folder</h3>
                        <p>Index any directory on your machine. Perfect for codebases, documentation, and notes.</p>
                        <span className="doc-badge available">Available</span>
                    </div>
                    <div className="doc-card-small">
                        <h3>📤 Uploads</h3>
                        <p>Drag & drop PDFs, Word docs, text files, and more. Great for reference materials.</p>
                        <span className="doc-badge available">Available</span>
                    </div>
                    <div className="doc-card-small">
                        <h3>🔗 Git Repository</h3>
                        <p>Clone and index a Git repo by URL.</p>
                        <span className="doc-badge coming">Coming Soon</span>
                    </div>
                    <div className="doc-card-small">
                        <h3>🌐 Web URL</h3>
                        <p>Crawl and index web pages and documentation sites.</p>
                        <span className="doc-badge coming">Coming Soon</span>
                    </div>
                </div>
            </section>

            <section className="doc-section">
                <h2>File Patterns</h2>
                <p>
                    Control which files get indexed using glob patterns. This is useful for excluding 
                    build artifacts, dependencies, and other noise.
                </p>
                
                <h3>Include Patterns</h3>
                <p>Specify which files to include (if empty, all files are included):</p>
                <pre className="doc-code"><code>*.ts, *.tsx, *.js, *.jsx
*.md, *.mdx
*.json, *.yaml, *.yml</code></pre>
                
                <h3>Exclude Patterns</h3>
                <p>Specify which files to skip:</p>
                <pre className="doc-code"><code>node_modules/**
dist/**
build/**
*.min.js
*.map
.git/**</code></pre>

                <div className="doc-note">
                    <strong>Tip:</strong> Linggen automatically respects <code>.gitignore</code> patterns, 
                    so you usually don't need to manually exclude common directories.
                </div>
            </section>

            <section className="doc-section">
                <h2>Re-indexing</h2>
                <p>
                    Click <strong>"Update"</strong> on any source to re-index after changes. 
                    Linggen uses incremental indexing for speed — only changed files are reprocessed.
                </p>
                <p>
                    You can also rename sources and edit patterns without re-indexing. 
                    The index will use the new settings on the next update.
                </p>
            </section>

            <section className="doc-section">
                <h2>Source Stats</h2>
                <p>Each source shows statistics after indexing:</p>
                <ul>
                    <li><strong>Files</strong> — Number of files indexed</li>
                    <li><strong>Chunks</strong> — Number of text chunks (each file is split into searchable chunks)</li>
                    <li><strong>Size</strong> — Total size of source content</li>
                    <li><strong>Last Indexed</strong> — When the source was last updated</li>
                </ul>
            </section>

            <section className="doc-section">
                <h2>Supported File Types</h2>
                <p>Linggen can extract text from:</p>
                <ul>
                    <li><strong>Code:</strong> .js, .ts, .py, .rs, .go, .java, .c, .cpp, .cs, .rb, .php, etc.</li>
                    <li><strong>Markup:</strong> .md, .mdx, .html, .xml, .rst</li>
                    <li><strong>Data:</strong> .json, .yaml, .yml, .toml, .csv</li>
                    <li><strong>Documents:</strong> .pdf, .docx, .doc, .txt</li>
                </ul>
            </section>
        </article>
    )
}

export default Sources
