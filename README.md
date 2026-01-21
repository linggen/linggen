<p align="center">
  <img src="frontend/public/logo.svg" width="120" alt="Linggen Logo">
</p>

# Linggen: Design Anchors for AI Coding.

**The alignment layer for your AI workflows.**

Linggen anchors your design decisions directly into your codebase so humans and AI can evolve it without losing its original shape. It bridges the "context gap" by providing persistent memory, a system-wide map, and a library of shared skills.

[Website](https://linggen.dev) • [Wiki](https://linggen.dev/wiki) • [Documentation](https://linggen.dev/docs)

---

## 🌀 Why Linggen?

Traditional AI chat is "blind" to anything you haven't manually copy-pasted. Linggen turns your codebase from a "black box" into a structured system that AI can actually understand:

- **🧠 Design Anchors (Memory):** Store architectural decisions, ADRs, and "tribal knowledge" in `.linggen/memory` as Markdown. AI recalls them via semantic search.
- **📊 System Map (Graph):** An Obsidian-like, zoomable dependency graph. Visualize file relationships and "blast radius" before you refactor.
- **🛠️ Shared Library & Skills:** Ingest pre-defined skills (e.g., `Software Architect`, `Senior Developer`, `React Expert`) to enforce consistency across projects and teams.
- **🔒 Local-First & Private:** All indexing and vector search (via LanceDB) happens on your machine. Your code and embeddings never leave your side.

---

## 🚀 Quick Start (macOS & Linux)

Install the CLI in seconds and start indexing:

```bash
curl -sSL https://linggen.dev/install-cli.sh | bash
linggen start
linggen index .
```

On Linux, you can set up the background server as a systemd service:

```bash
sudo linggen install
```

---

## 💬 How to use it with your AI

Linggen provides a Model Context Protocol (MCP) server that connects your local "brain" to MCP-enabled IDEs like **Cursor**, **Zed**, or **Claude Desktop**.

### Example Prompts:

> "Call Linggen MCP, find out how project-sender sends out messages, and summarize the architecture."

> "Load the 'Senior Developer' skill from Linggen and refactor this component to follow our clean code standards."

> "Check Linggen memory for any ADRs related to our database choice before suggesting a schema change."

---

## 📂 The Linggen Ecosystem

- **[linggen](https://github.com/linggen/linggen):** The core engine, CLI, and local server.
- **[linggen-vscode](https://github.com/linggen/linggen-vscode):** VS Code extension for Graph View and automatic MCP setup.
- **[Library Templates](backend/api/library_templates):** Pre-defined skills and policies to align your AI's behavior.

---

## 📜 License & Support

Linggen is open-source under the **[MIT License](LICENSE)**.

- **100% Free for Individuals:** Use it for all your personal and open-source projects.
- **Commercial Support:** For teams (5+ users) or companies, please support development by purchasing a **Commercial License**.

For more details, visit our [Pricing Page](https://linggen.dev/pricing) or [get in touch via email](mailto:linggen77@gmail.com).

---

## 🗺️ Roadmap

- [x] **Core Engine:** Local indexing and semantic search (LanceDB).
- [x] **MCP Support:** Use with Cursor, Zed, and Claude.
- [x] **Visual System Map:** Obsidian-like graph visualization of your codebase.
- [x] **Library System:** Shared skills and architecture policies.
- [ ] **Team Memory Sync:** Share architectural decisions across your team.
- [ ] **Deep Integration:** More IDEs and specialized agents.
- [ ] **Windows Support:** Bringing the local engine to more platforms.

MIT © 2026 [Linggen](https://linggen.dev)
