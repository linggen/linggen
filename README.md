<p align="center">
  <img src="frontend/public/logo.svg" width="120" alt="Linggen Logo">
</p>

# Linggen: Stop re-explaining to AI.

**The free and local app for your AI’s memory.**

Linggen indexes your codebases and tribal knowledge so your AI (Cursor, Zed, Claude, etc.) can actually understand your architecture, cross-project dependencies, and long-term decisions.

[Website](https://linggen.dev) • [VS Code Extension](https://marketplace.visualstudio.com/items?itemName=linggen.linggen-vscode) • [Documentation](https://linggen.dev/docs)

---

## 🌀 Why Linggen?

Traditional AI chat is "blind" to anything you haven't manually copy-pasted. Linggen bridges this "context gap" by providing:

- **🧠 Persistent Memory:** Store architectural decisions in `.linggen/memory` as Markdown. AI recalls them via semantic search.
- **🌐 Cross-Project Intelligence:** Work on Project A while your AI learns design patterns or auth logic from Project B.
- **📊 System Map (Graph):** Visualize file dependencies and "blast radius" before you refactor.
- **🔒 Local-First & Private:** All indexing and vector search (via LanceDB) happens on your machine. Your code and embeddings never leave your side. No accounts required.

---

## 🚀 Quick Start (macOS)

Install the CLI in seconds and start indexing:

```bash
curl -sSL https://linggen.dev/install-cli.sh | bash
linggen start
linggen index .
```

_Windows & Linux support coming soon._

---

## 💬 How to use it with your AI

Once Linggen is running and your project is indexed, simply talk to your MCP-enabled IDE (like Cursor or Zed):

> "Call Linggen MCP, find out how project-sender sends out messages, and ingest it."

> "Call Linggen MCP, load memory from Project-B, learn its code style and design pattern."

> "Load memory from Linggen, find out what is the goal of this piece of code."

---

## 📂 The Linggen Ecosystem

- **[linggen](https://github.com/linggen/linggen):** The core engine and CLI runtime.
- **[linggen-vscode](https://github.com/linggen/linggen-vscode):** VS Code extension for Graph View and automatic MCP setup.
- **[linggensite](https://github.com/linggen/linggensite):** (This Repo) The landing page and documentation site.
- **[linggen-releases](https://github.com/linggen/linggen-releases):** Pre-built binaries and distribution scripts.

---

## 📜 License & Support

Linggen is open-source under the **[MIT License](LICENSE)**.

- **100% Free for Individuals:** Use it for all your personal and open-source projects.
- **Local-First:** Your code and your "memory" never leave your machine.
- **Commercial Support:** If you are a team (5+ users) or a company using Linggen in a professional environment, we ask that you support the project's development by purchasing a **Commercial License**.

For more details on future enterprise features (SSO, Team Sync, RBAC), visit our [Pricing Page](https://linggen.dev/pricing) or [get in touch via email](mailto:linggen77@gmail.com).

---

## 🗺️ Roadmap

- [x] **Core Engine:** Local indexing and semantic search (LanceDB).
- [x] **MCP Support:** Use with Cursor, Zed, and Claude.
- [x] **Visual System Map:** Graph visualization of your codebase.
- [ ] **Team Memory Sync:** Share architectural decisions across your team.
- [ ] **Deep Integration:** More IDEs and specialized agents.
- [ ] **Windows Support:** Bringing the local engine to more platforms.

MIT © 2025 [Linggen](https://linggen.dev)
