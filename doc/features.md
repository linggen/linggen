# RememberMe Features

## 1. Universal Ingestion Engine
- [ ] **Git Integration**: Index full repositories, tracking commits and branches.
- [x] **Local Filesystem Watcher**: Real-time indexing of local folders (Obsidian vaults, project docs).
- [ ] **Web Crawler**: Index documentation sites (e.g., Rust docs, React docs) for offline RAG.

## 2. The "Brain" (Storage & Retrieval)
- **Hybrid Search**: Combine semantic search (vector) with keyword search (BM25) for precision.
- **Code Understanding**: Specialized chunking for code files (keeping functions together).
- **Long-term Memory**: Store TBs of history on disk using LanceDB without RAM bloat.

## 3. Interfaces
- **Desktop Dashboard**: Manage sources, view stats, manual query.
- **Chat API**: Standard API for chat apps to query "context".
- **IDE Bridge**: VS Code extension to "Chat with your codebase".

## 4. Privacy & Performance
- **100% Local**: No data leaves the machine.
- **BYO-Model**: Support local models (Llama3, Bert) or API-based (OpenAI/Claude) if user chooses.
