function Search() {
    return (
        <article className="doc-article">
            <h1>🔍 Search & AI Assistant</h1>
            <p className="doc-intro">
                Get the most out of Linggen's semantic search and AI chat capabilities.
            </p>

            <section className="doc-section">
                <h2>How Semantic Search Works</h2>
                <p>
                    Unlike keyword search, Linggen uses <strong>vector embeddings</strong> to find 
                    content by meaning. This means you can ask natural language questions and get 
                    relevant results even if your query doesn't match exact words in the documents.
                </p>
                <p>
                    When you index a source, Linggen:
                </p>
                <ol className="doc-steps">
                    <li>Splits files into chunks (paragraphs or logical sections)</li>
                    <li>Converts each chunk into a vector embedding using a local model</li>
                    <li>Stores embeddings in a local vector database (LanceDB)</li>
                </ol>
                <p>
                    When you search, your query is also converted to a vector, and Linggen finds 
                    the most similar chunks.
                </p>
            </section>

            <section className="doc-section">
                <h2>Search Tips</h2>
                <p>For best results:</p>
                
                <h3>Be Specific</h3>
                <div className="doc-compare">
                    <div className="doc-bad">
                        <span className="label">❌ Too vague</span>
                        <p>"How does it work?"</p>
                    </div>
                    <div className="doc-good">
                        <span className="label">✅ Better</span>
                        <p>"How does user authentication work in the login module?"</p>
                    </div>
                </div>

                <h3>Use Domain Terms</h3>
                <p>
                    Use terminology from your project. If your codebase calls it "UserSession", 
                    search for "UserSession" not "login state".
                </p>

                <h3>Mention Context</h3>
                <div className="doc-compare">
                    <div className="doc-bad">
                        <span className="label">❌ Missing context</span>
                        <p>"database connection"</p>
                    </div>
                    <div className="doc-good">
                        <span className="label">✅ Better</span>
                        <p>"PostgreSQL database connection in the backend API"</p>
                    </div>
                </div>
            </section>

            <section className="doc-section">
                <h2>AI Assistant</h2>
                <p>
                    The <strong>AI Assistant</strong> tab lets you have a conversation with context 
                    from your indexed sources. The AI automatically searches your knowledge base 
                    to answer questions.
                </p>

                <h3>Local LLM</h3>
                <p>
                    Enable the local LLM in <strong>Settings</strong> for fully offline operation. 
                    Linggen will download a small language model (~2GB) that runs entirely on your machine.
                </p>

                <h3>Example Questions</h3>
                <ul className="doc-examples">
                    <li>"Explain the architecture of this project"</li>
                    <li>"What design patterns are used in the authentication module?"</li>
                    <li>"How do I add a new API endpoint?"</li>
                    <li>"What's the purpose of the ConfigManager class?"</li>
                    <li>"Find all places where we handle errors"</li>
                </ul>
            </section>

            <section className="doc-section">
                <h2>Search Scope</h2>
                <p>
                    By default, search queries all indexed sources. You can narrow the scope:
                </p>
                <ul>
                    <li><strong>Source filter:</strong> Click on a source to search only within it</li>
                    <li><strong>File type:</strong> Mention file types in your query ("in TypeScript files")</li>
                </ul>
            </section>

            <section className="doc-section">
                <h2>Understanding Results</h2>
                <p>Search results show:</p>
                <ul>
                    <li><strong>Relevance score</strong> — How closely the chunk matches your query</li>
                    <li><strong>Source</strong> — Which indexed source the result comes from</li>
                    <li><strong>File path</strong> — The specific file containing the content</li>
                    <li><strong>Content preview</strong> — The matching text chunk</li>
                </ul>
                <p>
                    Higher scores (closer to 1.0) indicate more relevant results.
                </p>
            </section>
        </article>
    )
}

export default Search
