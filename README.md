# RememberMe

**RememberMe** is a local, privacy-focused RAG (Retrieval-Augmented Generation) service written in Rust. It turns your local history (git repos, docs, notes) into a searchable "second brain" for your AI tools.

## Documentation

- **[Features](doc/features.md)**: Detailed list of capabilities.
- **[Framework Architecture](doc/framework.md)**: System design and architecture diagram.

## Current Status
- **Frontend**: React + Vite setup, connected to backend.
- **Backend**: Rust Axum server, CORS enabled.
- **Ingestion**: Basic file walker and watcher implemented.

## Quick Start

### Prerequisites
- Rust (latest stable)
- Node.js & npm

### Running the Project
1. **VS Code**: Open the "Run and Debug" tab and select **"Full Stack"**.
2. **Manual**:
   - Backend: `cd backend && cargo run -p api`
   - Frontend: `cd frontend && npm run dev`

Access the frontend at `http://localhost:5173`.

## License

MIT
