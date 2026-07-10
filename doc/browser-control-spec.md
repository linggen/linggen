---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
status: v1 built + verified e2e; v2 MCP endpoint built + verified (Claude Code connects) — extension-side gate and Web Store listing pending
---

# Browser Control

Lets a Linggen agent **operate** the user's Chrome — navigate, read a page, click, type, scroll, screenshot — the way Claude in Chrome does. It extends the read-only [Browser Bridge](browser-bridge-spec.md) from "read a logged-in session" into a full perceive→act loop, so any skill (not just Pulse) can drive the browser to complete a task.

This is a deliberate scope expansion. The bridge was locked read-only by design ("we want its read-from-session mechanism, not its automation suite"). Browser Control reverses that for a new, separately-gated capability — it does **not** change how the read modules behave.

## Related docs

- `browser-bridge-spec.md` — the transport, broker, and frame protocol this reuses verbatim.
- `permission-spec.md` — the tool-permission model this plugs a new action class into.
- `tool-spec.md` — how built-in engine tools are defined and exposed to agents.
- Reference: Claude in Chrome (extension holds the browser-control capability; a local process drives it; the agent runs the loop). We match its shape, not its code.

## Why not MCP

The daemon ↔ extension link is **not** an agent-to-tools boundary — it is Linggen's own two components talking. MCP exists to standardize the agent-to-tools interface *across vendors*; wrapping the existing WebSocket in it buys nothing. So:

- **v1 — built-in engine tools over the existing WS bridge. No MCP.** Browser control is a set of native `Browser_*` tools that sit beside Read/Bash/Task and broker over the bridge. They inherit the permission system and session scoping for free.
- **v2 — a daemon-hosted MCP front door** so other vendors' agents (Claude Code, Cursor…) can drive the same Chrome. See *MCP front door* below. The MV3 extension cannot host a server (no listening sockets; the worker suspends), so the MCP surface lives in the daemon and fronts the same bridge.

## Model

A single **controlled tab** — visible, so the user watches what the agent does (unlike the read bridge's hidden throwaway tabs). The agent works a loop: read the page → decide an action → act → read again.

```
agent ──tool call──> engine Browser_* tools ──WS req──> extension ──CDP──> controlled tab
   Browser_readPage / Browser_click / Browser_type / Browser_screenshot / …
```

**Targeting is reference-first, pixels as fallback.** `Browser_readPage` returns an accessibility/DOM tree where each actionable node carries a stable `ref`. The agent clicks and types by `ref` — robust, cheap, and a good fit for a general model like gpt-5.5. `Browser_screenshot` plus pixel coordinates is the fallback for canvas/visual cases the tree can't express.

## Engine tools

Native tools, gated per the permission model (below). Each brokers one control op over the bridge.

| Tool | Purpose | Mutating? |
|:-----|:--------|:---------:|
| `Browser_navigate` | Load a URL / go back or forward in the controlled tab | yes |
| `Browser_readPage` | Return the accessibility/DOM tree with per-node `ref`s | no |
| `Browser_screenshot` | Capture the controlled tab (or a region) | no |
| `Browser_click` | Click a node by `ref` (or coordinate) | yes |
| `Browser_type` | Type text into the focused / referenced field | yes |
| `Browser_key` | Press a key or chord | yes |
| `Browser_scroll` | Scroll the page or an element | no |
| `Browser_wait` | Wait for load / a selector / a delay before the next read | no |
| `Browser_readConsole` | Read console messages (debugging) | no |
| `Browser_tabs` | List / open / switch the controlled tab set | `open` only |

Read-class tools (`readPage`, `screenshot`, `scroll`, `readConsole`) never mutate site state; mutating tools go through the safety gate.

## Control module

A new bridge module `control` (id `control`), alongside the read modules. Its ops carry the browser-control protocol over the **same** frame contract as the X module.

| op | params | notes |
|:---|:-------|:------|
| `navigate` | `{ url \| "back" \| "forward", tab? }` | resolves after load settles |
| `read_page` | `{ tab?, filter?, depth? }` | returns node tree with `ref` ids |
| `screenshot` | `{ tab?, region? }` | returns an image, base64 |
| `click` | `{ ref \| coordinate, button?, modifiers? }` | |
| `type` | `{ text, ref? }` | types into focus or `ref` |
| `key` | `{ keys, repeat? }` | key or chord |
| `scroll` | `{ direction, amount, ref? }` | |
| `wait` | `{ for: "load" \| "selector" \| "ms", value }` | settle before the next read |
| `tabs` | `{ action: "list" \| "open" \| "switch" \| "close", … }` | manages the controlled tab set |

Results reuse the bridge envelope: `{ ok, data }` or `{ ok:false, code, message }`, with the same `code` vocabulary plus `not_permitted` (the safety gate declined) and `element_gone` (a stale `ref`).

### Frames

No new transport. The existing `req`/`res` frames carry `module: "control"`:

```
daemon → ext   { "t":"req", "id":"…", "module":"control", "op":"click", "params":{ "ref":"n42" } }
ext → daemon   { "t":"res", "id":"…", "ok":true, "data":{ "clicked":true } }
```

The handshake gains the `control` module in the extension's `hello` so the daemon knows control is available and which version.

## Execution (extension side)

The extension drives the controlled tab through the **Chrome DevTools Protocol** (`chrome.debugger`): input via CDP `Input`, capture via `Page`, tree via the `Accessibility`/`DOM` domains. This is the faithful path Claude in Chrome and Playwright use — real events, true screenshots — and it is why control needs `debugger` plus host access the read bridge never required. CDP attaching surfaces Chrome's "started debugging this browser" banner; that is expected and disclosed, not a defect.

## Permission and safety

The gate lives in the **extension** (v2, aligned with Claude in Chrome) so every caller — Linggen sessions and `/mcp` agents alike — passes the same floor:

- **Read is free.** `readPage` / `screenshot` / `scroll` / `readConsole` run without a prompt.
- **Mutating actions gate per origin.** `navigate`, `click`, `type`, `key`, `tabs open` on an untrusted origin pop a small extension window on the controlled tab: *Always allow this site* (persists in extension storage until revoked from the popup), *Allow once* (this browsing session), *Deny*. Trusted origins run without prompts; a closed or unanswered prompt (120s) is a deny (`not_permitted`).
- **A hard floor never auto-executes**, even on a trusted site — payment, passwords/security, deleting data, posting/sending on the user's behalf. Floor prompts never offer "Always" and persist nothing. Recognized from the target's accessible name, so it covers ref-targeted actions; a coordinate click has no name to inspect — another reason refs are the preferred targeting mode.
- **The controlled tab is visible**, and the agent's actions are legible in it, so the user can interrupt.
- The prompt is an extension window, not page DOM — the page can't render, click, or dismiss it.
- Interim: the engine's per-session gate (`browser_origins`, in-chat prompt) is still active for Linggen sessions and retires once the extension gate is verified live — the system is never gateless in between.

## MCP front door (v2)

Browser control as a product for *any* agent, not just Linggen's. The daemon hosts an MCP server; the extension stays the hands. Two mouths, one brain: native `Browser_*` tools (Linggen sessions) and the MCP endpoint (third-party agents) front the same bridge broker.

- **Endpoint** — streamable-HTTP MCP server on the daemon (`http://127.0.0.1:9898/mcp`), localhost-only. Tools mirror the control ops one-to-one: `browser_navigate`, `browser_read_page`, `browser_click`, `browser_type`, `browser_key`, `browser_scroll`, `browser_screenshot`, `browser_wait`, `browser_tabs`, `browser_read_console`.
- **Linggen agents stay on the native tools.** The tool name is already the switchable seam; the native path carries what MCP cannot — gate prompts in the calling chat, per-session `browser_origins`, screenshots attached to the conversation. (Claude Code makes the same call: Claude in Chrome is built into its CLI, not an MCP entry.)
- **Install story for other agents** — daemon (`install.sh`) + extension (Web Store) + one line in the agent's MCP config. Two installs is the floor for any real-browser tool: MV3 forbids the extension being the server, so some OS-side process must exist.
- **The gate lives in the extension** (see *Permission and safety*): the Allow prompt renders in the browser, trust persists in extension storage until revoked. The extension gate is the floor every caller passes — no agent gets a bypass.
- **Session reads ride the same endpoint**: `x_search` / `x_targets` / `x_following` / `x_whotofollow` / `x_own` expose the x module's structured logged-in reads to any MCP agent — the capability no generic browser MCP has.
- **Open** — whether MCP callers and Linggen sessions share one controlled tab or get one each.

## Distribution

Public Chrome Web Store, same channel as the read extension — Pulse-style capability-probe + deep-link to install; it cannot ship via `install.sh`. A control extension is a harder review than the x.com-only reader:

- `debugger` permission and broad host access draw single-purpose scrutiny — the listing must state the browser-control purpose plainly.
- Prefer **per-site opt-in** (activeTab / on-request host grants) over `<all_urls>` up front.
- The debugging banner is part of the UX; document it for users so it does not read as malware.

## Phasing

- **v1** — controlled-tab loop with the tool set above, reference-first targeting, the site-trust gate, CDP execution, Web Store listing. Built; verified end-to-end (Chrome and Arc).
- **v2** — the MCP front door (endpoint built — `/mcp`, stateless streamable HTTP, ten `browser_*` tools) + extension-side gate and trust list (open).
- **Deferred** — multi-tab orchestration beyond one controlled tab; non-Chromium browsers.

## Out of scope

- Driving the read modules' hidden tabs (they stay read-only, on-demand).
- Background / headless automation with no visible tab and no user present.
- Bypassing bot-detection or CAPTCHAs.
