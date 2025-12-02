import { useState } from 'react'

function UsageGuide() {
    const [activeTab, setActiveTab] = useState('quickstart')

    const tabs = [
        { id: 'quickstart', label: '🚀 Quick Start' },
        { id: 'sources', label: '📂 Adding Sources' },
        { id: 'mcp', label: '🔌 MCP Setup' },
        { id: 'search', label: '🔍 Search & Chat' },
    ]

    const content = {
        quickstart: {
            title: 'Quick Start Guide',
            sections: [
                {
                    heading: '1. Download & Install',
                    content: `Download Linggen from GitHub Releases. Open the .dmg file and drag Linggen to your Applications folder.`,
                    code: null
                },
                {
                    heading: '2. First Launch',
                    content: `On first launch, Linggen will download the embedding model (~100MB). This only happens once. The app runs a local backend at localhost:8787.`,
                    code: null
                },
                {
                    heading: '3. Add Your First Source',
                    content: `Go to the Sources tab and click "Add Source". Choose "Local Folder" and select a project directory. Click "Index" to start building the vector index.`,
                    code: null
                },
                {
                    heading: '4. Start Searching',
                    content: `Once indexing completes, go to the AI Assistant tab. Ask questions about your codebase in natural language.`,
                    code: null
                }
            ]
        },
        sources: {
            title: 'Adding & Managing Sources',
            sections: [
                {
                    heading: 'Source Types',
                    content: `Linggen supports multiple source types:`,
                    list: [
                        'Local Folder — Index any directory on your machine',
                        'Uploads — Drag & drop PDFs, docs, and text files',
                        'Git Repo — Coming soon',
                        'Web URL — Coming soon'
                    ]
                },
                {
                    heading: 'File Patterns',
                    content: `Control which files get indexed using glob patterns:`,
                    code: `Include: *.ts, *.tsx, *.md, *.json
Exclude: node_modules/**, *.min.js, dist/**`
                },
                {
                    heading: 'Re-indexing',
                    content: `Click "Update" on any source to re-index after changes. Linggen uses incremental indexing for speed.`,
                    code: null
                }
            ]
        },
        mcp: {
            title: 'MCP Server Setup',
            sections: [
                {
                    heading: 'What is MCP?',
                    content: `Model Context Protocol (MCP) lets AI tools like Cursor access Linggen's knowledge base. Linggen runs an MCP server automatically.`,
                    code: null
                },
                {
                    heading: 'Connect Cursor IDE',
                    content: `Open Cursor Settings → MCP → Add Server:`,
                    code: `Name: linggen
URL: http://localhost:8787/mcp/sse`
                },
                {
                    heading: 'Team Setup',
                    content: `For team use, run Linggen on a shared machine and point team members' Cursor to that IP:`,
                    code: `URL: http://192.168.1.100:8787/mcp/sse`
                },
                {
                    heading: 'Available Tools',
                    content: `The MCP server exposes these tools to your AI:`,
                    list: [
                        'search — Semantic search across all indexed sources',
                        'list_sources — View available knowledge sources',
                        'get_context — Retrieve relevant context for a query'
                    ]
                }
            ]
        },
        search: {
            title: 'Search & AI Assistant',
            sections: [
                {
                    heading: 'Semantic Search',
                    content: `Linggen uses vector embeddings to find relevant content by meaning, not just keywords. Ask questions like:`,
                    list: [
                        '"How does authentication work in this project?"',
                        '"Where is the database connection configured?"',
                        '"Show me examples of error handling"'
                    ]
                },
                {
                    heading: 'AI Chat',
                    content: `The AI Assistant tab lets you have a conversation with context from your indexed sources. Enable the local LLM in Settings for fully offline operation.`,
                    code: null
                },
                {
                    heading: 'Search Tips',
                    content: `For best results:`,
                    list: [
                        'Be specific about what you\'re looking for',
                        'Mention file types or areas of the codebase',
                        'Use domain-specific terminology from your project'
                    ]
                }
            ]
        }
    }

    const currentContent = content[activeTab]

    return (
        <section className="guide-section" id="docs">
            <div className="container">
                <h2 className="section-title">
                    <span className="title-decoration">◆</span>
                    Documentation
                    <span className="title-decoration">◆</span>
                </h2>
                <p className="section-description">
                    Learn how to get the most out of Linggen
                </p>

                <div className="guide-container">
                    <div className="guide-tabs">
                        {tabs.map(tab => (
                            <button
                                key={tab.id}
                                className={`guide-tab ${activeTab === tab.id ? 'active' : ''}`}
                                onClick={() => setActiveTab(tab.id)}
                            >
                                {tab.label}
                            </button>
                        ))}
                    </div>

                    <div className="guide-content">
                        <h3 className="guide-title">{currentContent.title}</h3>
                        
                        {currentContent.sections.map((section, index) => (
                            <div key={index} className="guide-card">
                                <h4>{section.heading}</h4>
                                <p>{section.content}</p>
                                
                                {section.list && (
                                    <ul className="guide-list">
                                        {section.list.map((item, i) => (
                                            <li key={i}>{item}</li>
                                        ))}
                                    </ul>
                                )}
                                
                                {section.code && (
                                    <pre className="guide-code">
                                        <code>{section.code}</code>
                                    </pre>
                                )}
                            </div>
                        ))}
                    </div>
                </div>
            </div>
        </section>
    )
}

export default UsageGuide
