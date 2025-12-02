function MCPSetup() {
    return (
        <article className="doc-article">
            <h1>🔌 MCP Server Setup</h1>
            <p className="doc-intro">
                Connect Linggen to AI coding tools like Cursor IDE via the Model Context Protocol.
            </p>

            <section className="doc-section">
                <h2>What is MCP?</h2>
                <p>
                    <strong>Model Context Protocol (MCP)</strong> is a standard that lets AI tools 
                    access external context sources. When you connect Cursor to Linggen's MCP server, 
                    the AI can search your indexed knowledge base to provide better answers.
                </p>
                <p>
                    Linggen runs an MCP server automatically at <code>localhost:8787/mcp/sse</code>.
                </p>
            </section>

            <section className="doc-section">
                <h2>Connect Cursor IDE</h2>
                <ol className="doc-steps">
                    <li>Make sure Linggen is running</li>
                    <li>Open <strong>Cursor → Settings → MCP</strong></li>
                    <li>Click <strong>"Add Server"</strong></li>
                    <li>Enter the following:</li>
                </ol>
                
                <pre className="doc-code"><code>{`Name: linggen
URL: http://localhost:8787/mcp/sse`}</code></pre>

                <ol className="doc-steps" start="5">
                    <li>Save and restart Cursor</li>
                </ol>

                <div className="doc-note">
                    <strong>Verify:</strong> In Cursor, the MCP indicator should show "linggen" as connected. 
                    You can test by asking Cursor about code in your indexed sources.
                </div>
            </section>

            <section className="doc-section">
                <h2>Team Setup</h2>
                <p>
                    For team use, run Linggen on a shared machine (or server) and have team members 
                    connect to that instance instead of localhost.
                </p>
                
                <h3>On the Server</h3>
                <ol className="doc-steps">
                    <li>Install and run Linggen on a machine accessible to your team</li>
                    <li>Index your shared codebase and documentation</li>
                    <li>Note the machine's IP address (e.g., <code>192.168.1.100</code>)</li>
                </ol>

                <h3>On Team Members' Machines</h3>
                <p>Configure Cursor to point to the shared server:</p>
                <pre className="doc-code"><code>{`Name: linggen-team
URL: http://192.168.1.100:8787/mcp/sse`}</code></pre>

                <div className="doc-note">
                    <strong>Security:</strong> The MCP server has no authentication by default. 
                    Only expose it on trusted networks or behind a VPN.
                </div>
            </section>

            <section className="doc-section">
                <h2>Available MCP Tools</h2>
                <p>The MCP server exposes these tools to connected AI clients:</p>
                
                <table className="doc-table">
                    <thead>
                        <tr>
                            <th>Tool</th>
                            <th>Description</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr>
                            <td><code>search</code></td>
                            <td>Semantic search across all indexed sources</td>
                        </tr>
                        <tr>
                            <td><code>list_sources</code></td>
                            <td>List available knowledge sources</td>
                        </tr>
                        <tr>
                            <td><code>get_context</code></td>
                            <td>Retrieve relevant context for a query</td>
                        </tr>
                    </tbody>
                </table>
            </section>

            <section className="doc-section">
                <h2>Troubleshooting</h2>
                
                <h3>Cursor doesn't connect</h3>
                <ul>
                    <li>Ensure Linggen is running (check <code>localhost:8787</code> in browser)</li>
                    <li>Restart Cursor after adding the MCP server</li>
                    <li>Check Cursor's MCP logs for error messages</li>
                </ul>

                <h3>Search results are empty</h3>
                <ul>
                    <li>Make sure you've indexed at least one source</li>
                    <li>Check that the source has completed indexing (not stuck at 0%)</li>
                    <li>Try a broader search query</li>
                </ul>
            </section>
        </article>
    )
}

export default MCPSetup
