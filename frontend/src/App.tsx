import { useState } from 'react'
import './App.css'
import { indexDocument, searchDocuments, type SearchResult } from './api'

function App() {
  // Index form state
  const [docId, setDocId] = useState('')
  const [content, setContent] = useState('')
  const [indexing, setIndexing] = useState(false)
  const [indexStatus, setIndexStatus] = useState('')

  // Search state
  const [query, setQuery] = useState('')
  const [searching, setSearching] = useState(false)
  const [results, setResults] = useState<SearchResult[]>([])
  const [searchError, setSearchError] = useState('')

  const handleIndex = async (e: React.FormEvent) => {
    e.preventDefault()
    setIndexing(true)
    setIndexStatus('')

    try {
      const response = await indexDocument({
        document_id: docId,
        content: content,
      })
      setIndexStatus(`✓ Indexed ${response.chunks_indexed} chunks for document: ${response.document_id}`)
      setDocId('')
      setContent('')
    } catch (error) {
      setIndexStatus(`✗ Error: ${error}`)
    } finally {
      setIndexing(false)
    }
  }

  const handleSearch = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!query.trim()) return

    setSearching(true)
    setSearchError('')

    try {
      const response = await searchDocuments(query, 10)
      setResults(response.results)
      if (response.results.length === 0) {
        setSearchError('No results found. Try indexing some documents first!')
      }
    } catch (error) {
      setSearchError(`Search failed: ${error}`)
      setResults([])
    } finally {
      setSearching(false)
    }
  }

  return (
    <div className="app">
      <header>
        <h1>🧠 RememberMe RAG</h1>
        <p>Local semantic search powered by Rust + Candle</p>
      </header>

      <div className="container">
        {/* Index Section */}
        <section className="section">
          <h2>📥 Index Document</h2>
          <form onSubmit={handleIndex}>
            <div className="form-group">
              <label htmlFor="docId">Document ID</label>
              <input
                id="docId"
                type="text"
                value={docId}
                onChange={(e) => setDocId(e.target.value)}
                placeholder="e.g., doc1, my-notes, etc."
                required
              />
            </div>
            <div className="form-group">
              <label htmlFor="content">Content</label>
              <textarea
                id="content"
                value={content}
                onChange={(e) => setContent(e.target.value)}
                placeholder="Paste your document content here..."
                rows={6}
                required
              />
            </div>
            <button type="submit" disabled={indexing}>
              {indexing ? 'Indexing...' : 'Index Document'}
            </button>
          </form>
          {indexStatus && (
            <div className={`status ${indexStatus.startsWith('✓') ? 'success' : 'error'}`}>
              {indexStatus}
            </div>
          )}
        </section>

        {/* Search Section */}
        <section className="section">
          <h2>🔍 Search</h2>
          <form onSubmit={handleSearch}>
            <div className="form-group">
              <label htmlFor="query">Search Query</label>
              <input
                id="query"
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="What are you looking for?"
                required
              />
            </div>
            <button type="submit" disabled={searching}>
              {searching ? 'Searching...' : 'Search'}
            </button>
          </form>

          {searchError && <div className="status error">{searchError}</div>}

          {results.length > 0 && (
            <div className="results">
              <h3>Results ({results.length})</h3>
              {results.map((result, idx) => (
                <div key={idx} className="result-card">
                  <div className="result-header">
                    <span className="doc-id">{result.document_id}</span>
                    <span className="score">Score: {result.score.toFixed(3)}</span>
                  </div>
                  <p className="result-content">{result.content}</p>
                </div>
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  )
}

export default App
