---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
status: v1 built — engine tools + extension control module; Web Store listing pending
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
- **Later, optional — a daemon-hosted MCP server** as a *front door* so other vendors' agents (Claude Code, Cursor…) can drive Linggen's Chrome. The MV3 extension cannot host a server; the MCP surface, if built, lives in the daemon and fronts the same bridge. This is a product play, not a dependency of v1.

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

Browser actions are a new tool class in `permission-spec.md`, distinct from filesystem modes. The default posture:

- **Read is free.** `readPage` / `screenshot` / `scroll` / `readConsole` run without a prompt.
- **Mutating actions are gated per-action until the site is trusted.** `navigate`, `click`, `type`, `key` each prompt for confirmation on an untrusted origin. The confirmation offers "allow this site for the session": once granted, mutating actions on that origin run without further prompts. A fresh origin re-prompts. (Decided: this over a task-scoped handover — safer default, and the site-grant keeps prompt fatigue bounded.)
- **A hard floor never auto-executes**, even on a trusted site — submitting payment, changing passwords/security settings, deleting data, posting/sending on the user's behalf. These pause for an explicit per-action confirmation, mirroring the assistant safety rules and Claude in Chrome's action categories. The floor is recognized from the target element's accessible name, so it applies to ref-targeted actions; a coordinate click has no name to inspect — another reason refs are the preferred targeting mode.
- Trusted origins are stored per session (`browser_origins` in the session's `permission.json`), alongside the path grants.
- **The controlled tab is visible**, and the agent's actions are legible in it, so the user can interrupt.

## Distribution

Public Chrome Web Store, same channel as the read extension — Pulse-style capability-probe + deep-link to install; it cannot ship via `install.sh`. A control extension is a harder review than the x.com-only reader:

- `debugger` permission and broad host access draw single-purpose scrutiny — the listing must state the browser-control purpose plainly.
- Prefer **per-site opt-in** (activeTab / on-request host grants) over `<all_urls>` up front.
- The debugging banner is part of the UX; document it for users so it does not read as malware.

## Phasing

- **v1** — controlled-tab loop with the tool set above, reference-first targeting, the site-trust gate, CDP execution, Web Store listing.
- **Deferred** — multi-tab orchestration beyond one controlled tab; a daemon-hosted MCP front door for third-party agents; non-Chromium browsers.

## Out of scope

- Driving the read modules' hidden tabs (they stay read-only, on-demand).
- Background / headless automation with no visible tab and no user present.
- Bypassing bot-detection or CAPTCHAs.
