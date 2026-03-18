# UI SDK v2 — Plan

## Problem

The current UI SDK (`ui-sdk/`, ~1,100 lines) is a minimal chat-only component. The main chat panel (`ui/src/components/chat/`, ~2,900 lines) has 11 more components that handle tool activity, permission prompts, plans, subagents, content blocks, etc. App skills like sys-doctor need these features but can't access them — the SDK doesn't provide them.

**Result**: skill developers must either build their own tool/permission UI (bad DX) or accept a degraded experience (no permission prompts, no tool output, no progress).

## Current State

### Main Chat (full-featured)

```
ui/src/components/chat/
├── ChatPanel.tsx          # Full message rendering, tool activity, plans
├── AgentMessage.tsx       # Rich agent response with content blocks
├── ContentBlockView.tsx   # Tool calls: Bash running, output, errors
├── SpecialBlocks.tsx      # Permission prompt, file preview
├── PlanBlock.tsx          # Plan mode rendering
├── SubagentDrawer.tsx     # Subagent tree panel
├── SubagentTreeView.tsx   # Nested agent visualization
├── ThinkingIndicator.tsx  # With elapsed time
├── ChatInput.tsx          # With slash commands, file attach
├── MarkdownContent.tsx    # Full markdown + code highlighting
├── TodoPanel.tsx          # Task tracking
├── MessagePhase.ts        # Message lifecycle transitions
├── MessageHelpers.ts      # Parsing utilities
└── types.ts
```

Plus stores: `chatStore`, `agentStore`, `uiStore`, `projectStore`
Plus hooks: `useSseConnection`, `useSseDispatch`
Plus lib: `sseEventHandlers`, `messageUtils`

### UI SDK v1 (minimal)

```
ui-sdk/src/
├── components/
│   ├── ChatPanel.tsx      # Basic message list + input
│   ├── ChatInput.tsx      # Plain text input
│   ├── MessageBubble.tsx  # Simple bubble
│   ├── MarkdownContent.tsx # Basic markdown
│   └── ThinkingIndicator.tsx # Basic dots
├── api/
│   ├── client.ts          # REST calls
│   └── sse.ts             # SSE connection
├── state/
│   └── chat-store.ts      # Message state
├── lib/
│   └── mount.ts           # Vanilla JS mount API
└── types.ts
```

## Design Options

### Option A: Extract main chat as SDK (recommended)

**Idea**: The main chat panel IS the SDK. Extract the chat components from `ui/src/components/chat/` into a shared package that both the main app and the SDK consume.

```
packages/
├── chat-components/       # Shared React components
│   ├── ChatPanel.tsx
│   ├── AgentMessage.tsx
│   ├── ContentBlockView.tsx
│   ├── SpecialBlocks.tsx   (PermissionPrompt, FilePreview)
│   ├── ThinkingIndicator.tsx
│   ├── ChatInput.tsx
│   ├── MarkdownContent.tsx
│   └── ...
├── chat-state/            # Shared state management
│   ├── chatStore.ts
│   ├── sseConnection.ts
│   └── eventHandlers.ts
└── chat-sdk/              # Mount API + UMD bundle
    ├── mount.ts           # LinggenUI.mount()
    └── vite.config.ts     # Builds UMD bundle
```

**Pros**:
- Zero duplication — one codebase, two consumers
- SDK always matches main chat features
- Bug fixes apply everywhere
- Main chat panel IS the SDK (battle-tested)

**Cons**:
- Refactoring effort to extract components
- Need to handle dependencies (main app stores vs SDK stores)
- Bundle size increases for SDK

### Option B: Keep separate, copy components

**Idea**: Manually port missing components from main chat to SDK.

**Pros**: Simpler, no architectural change
**Cons**: Perpetual drift, double maintenance, SDK always lags behind

### Option C: Iframe the main chat panel

**Idea**: The SDK mounts an iframe pointing to a special route like `/sdk/chat?session=X&skill=Y`.

**Pros**: Zero code duplication, always up to date
**Cons**: Cross-origin complexity, styling constraints, harder to hook into (onStreamEnd, etc.)

## Recommendation: Option A (shared package)

The main chat is already React. The SDK is already React (bundled with Vite into UMD). The only difference is how state is managed — main app uses global Zustand stores, SDK uses its own internal store.

**Key insight**: The SDK doesn't need all the main app's stores. It needs:
- SSE connection (scoped to one session)
- Message list state
- Pending permission state
- Content block state

These can be a subset of the main stores, extracted into a shared module.

## Implementation Plan

### Phase 1: Extract shared chat components (core)

Move components from `ui/src/components/chat/` into a shared location that both the main app and SDK can import.

**Approach**: Create `ui/src/components/chat-core/` with the portable components. The main chat imports from there. The SDK also imports from there (via Vite aliasing or copy at build time).

Components to share:
1. `AgentMessage.tsx` — agent response rendering with content blocks
2. `ContentBlockView.tsx` — tool execution display (Bash, Read, Write, etc.)
3. `SpecialBlocks.tsx` — **permission prompt**, file preview
4. `MarkdownContent.tsx` — full markdown with code highlighting
5. `ThinkingIndicator.tsx` — with elapsed time
6. `MessagePhase.ts` — phase transitions
7. `MessageHelpers.ts` — parsing utilities

NOT shared (main-app-specific):
- `PlanBlock.tsx` — plan mode (not needed in SDK v2)
- `SubagentDrawer/TreeView.tsx` — subagent visualization (future)
- `TodoPanel.tsx` — task tracking (future)

### Phase 2: SDK state management

Replace SDK's simple `chat-store.ts` with a richer state that handles:
- Messages with content blocks (not just text)
- Pending permission prompts (ask_user)
- Tool activity tracking (which tools are running)
- Streaming state (thinking, content tokens)

This is a new `sdk-store.ts` that mirrors the relevant parts of `chatStore` + `uiStore`.

### Phase 3: SDK SSE handling

Upgrade SDK's `sse.ts` to handle all event types the main chat handles:
- `token` — streaming text (already works)
- `content_block` — tool execution start/update (new)
- `ask_user` — permission prompts (new)
- `text_segment` — text segments (new)
- `turn_complete` — end of turn (already works)
- `activity` — agent status (already works)

### Phase 4: Permission prompt in SDK

Add permission rendering to SDK:
1. SDK receives `ask_user` SSE event
2. Renders permission dialog inline in the chat
3. User clicks Allow/Deny
4. SDK calls `POST /api/ask-user-response`
5. Agent continues

This is the critical feature for sys-doctor.

### Phase 5: Update mount API

Expand `ChatPanelOptions` with new callbacks:

```typescript
interface ChatPanelOptions {
  // Existing
  onSessionCreated?: (sessionId: string) => void;
  onMessage?: (message: ChatMessage) => void;
  onStreamToken?: (fullText: string) => void;
  onStreamEnd?: (text: string) => void;

  // New in v2
  onToolStart?: (tool: string, args: any) => void;
  onToolEnd?: (tool: string, result: string) => void;
  onPermissionRequest?: (question: PermissionQuestion) => void;
  onPermissionResponse?: (questionId: string, answer: string) => void;

  // Display options
  showToolActivity?: boolean;    // default true
  showPermissions?: boolean;     // default true
  showContentBlocks?: boolean;   // default true
}
```

### Phase 6: Bundle optimization

The shared components add weight. Optimize:
- Tree-shake unused components
- Lazy-load code highlighting (heaviest dep)
- Keep CSS injection (no external stylesheets)
- Target: < 150KB gzipped (currently ~40KB)

## File Changes Summary

```
ui/src/components/chat-core/    # NEW: shared components
├── AgentMessage.tsx             # extracted from chat/
├── ContentBlockView.tsx         # extracted from chat/
├── SpecialBlocks.tsx            # extracted from chat/
├── MarkdownContent.tsx          # extracted from chat/
├── ThinkingIndicator.tsx        # extracted from chat/
├── MessagePhase.ts              # extracted from chat/
├── MessageHelpers.ts            # extracted from chat/
└── types.ts                     # shared types

ui/src/components/chat/          # UPDATED: imports from chat-core/
├── ChatPanel.tsx                # uses chat-core/ components
└── ChatInput.tsx                # main-app-specific (slash commands)

ui-sdk/src/
├── components/
│   ├── ChatPanel.tsx            # REWRITTEN: uses chat-core/ components
│   ├── ChatInput.tsx            # SIMPLIFIED: no slash commands
│   └── PermissionPrompt.tsx     # NEW: permission dialog
├── state/
│   └── sdk-store.ts             # REWRITTEN: richer state
├── api/
│   ├── client.ts                # UPDATED: add ask-user-response
│   └── sse.ts                   # UPDATED: handle all event types
└── lib/
    └── mount.ts                 # UPDATED: new options
```

## Priority Order

1. **Permission prompt** — unblocks sys-doctor and any skill using Bash
2. **Content blocks** — shows tool activity (Bash running, output)
3. **Agent message rendering** — proper formatting
4. **State management** — richer store
5. **Bundle optimization** — size reduction

## Timeline Estimate

- Phase 1-2 (extract + state): Foundation work
- Phase 3-4 (SSE + permissions): Critical for sys-doctor
- Phase 5-6 (API + optimization): Polish

## Revised Recommendation: Option C (iframe) + thin SDK wrapper

After further analysis, **Option C is the best approach** — simpler, zero maintenance, full feature parity.

### Why iframe wins

The VSCode extension already embeds the Linggen chat via iframe/webview. The server already sets `X-Frame-Options` to allow it. Same-origin means no CORS issues.

The key insight: **we don't need to extract or duplicate components at all**. We just need a server route that renders the chat panel standalone (no sidebar, no session list — just the chat), and a thin SDK wrapper that creates the iframe and bridges events via `postMessage`.

### Architecture

```
Skill page (developer's HTML/JS)
│
├── Dashboard area (developer builds this)
│
└── <div id="chat-panel">
        ↓ LinggenUI.mount(el, opts)
        ↓
    ┌─────────────────────────────────┐
    │ <iframe src="/embed/chat        │
    │   ?session_id=X                 │
    │   &skill=sys-doctor             │
    │   &model=gpt-5.4">             │
    │                                 │
    │  Full main chat panel           │
    │  • Permission prompts ✓         │
    │  • Tool activity ✓              │
    │  • Content blocks ✓             │
    │  • Markdown ✓                   │
    │  • Thinking indicator ✓         │
    │  • Everything ✓                 │
    │                                 │
    └─────────────────────────────────┘
        ↑ postMessage events
        ↑
    Parent window receives:
    • onStreamEnd(text)
    • onSessionCreated(id)
    • onToolStart(tool, args)
    • etc.
```

### Server: `/embed/chat` route

A new route that serves a minimal HTML page with just the ChatPanel component:

```html
<!-- /embed/chat?session_id=X&skill=Y&model=Z -->
<!DOCTYPE html>
<html>
<head>
  <style>
    body { margin: 0; height: 100vh; overflow: hidden; }
    #root { height: 100%; }
  </style>
</head>
<body>
  <div id="root"></div>
  <script src="/assets/embed-chat.js"></script>
</body>
</html>
```

`embed-chat.js` is a small entry point that:
1. Reads query params (session_id, skill, model)
2. Renders the main ChatPanel in embed mode (no header, no sidebar)
3. Bridges events to parent via `postMessage`

### SDK v2: Thin wrapper

The SDK becomes very small — just iframe management + postMessage bridge:

```javascript
// linggen-ui.umd.js (< 5KB, no React needed!)

const LinggenUI = {
  mount(container, options) {
    const iframe = document.createElement('iframe');
    const params = new URLSearchParams({
      skill: options.skillName || '',
      model: options.modelId || '',
      session_id: options.sessionId || '',
      title: options.title || '',
      placeholder: options.placeholder || '',
    });
    iframe.src = `/embed/chat?${params}`;
    iframe.style.cssText = 'width:100%;height:100%;border:none;';
    container.appendChild(iframe);

    // Listen for postMessage from iframe
    window.addEventListener('message', (e) => {
      if (e.source !== iframe.contentWindow) return;
      const { type, payload } = e.data;
      switch (type) {
        case 'session_created': options.onSessionCreated?.(payload.sessionId); break;
        case 'stream_token':    options.onStreamToken?.(payload.text); break;
        case 'stream_end':      options.onStreamEnd?.(payload.text); break;
        case 'message':         options.onMessage?.(payload); break;
      }
    });

    return {
      send(text) { iframe.contentWindow.postMessage({ type: 'send', text }, '*'); },
      addMessage(role, text) { iframe.contentWindow.postMessage({ type: 'add_message', role, text }, '*'); },
      clear() { iframe.contentWindow.postMessage({ type: 'clear' }, '*'); },
      destroy() { container.removeChild(iframe); },
      getSessionId() { /* stored from session_created event */ },
      setOptions(opts) { iframe.contentWindow.postMessage({ type: 'set_options', opts }, '*'); },
    };
  }
};
```

### Benefits

1. **Full feature parity** — iframe runs the real chat. Permissions, tools, content blocks, everything.
2. **Zero maintenance** — no separate SDK components to maintain. Main app updates = SDK updates.
3. **Tiny SDK** — < 5KB (just iframe + postMessage). No React bundled.
4. **Same API** — `LinggenUI.mount()`, `chat.send()`, callbacks — backward compatible.
5. **Works with any framework** — skill can be vanilla JS, React, Vue, Svelte, whatever.

### Migration path

1. Keep current SDK as-is (backward compatible)
2. Add `/embed/chat` server route
3. New SDK v2 uses iframe internally
4. `LinggenUI.mount()` API stays the same — old skills keep working

### Implementation tasks

1. **Server route**: Add `/embed/chat` that renders ChatPanel in embed mode
2. **Embed entry point**: `embed-chat.tsx` — minimal React app with ChatPanel + postMessage bridge
3. **postMessage protocol**: Define message types (send, add_message, stream_end, session_created, etc.)
4. **SDK wrapper**: Rewrite `linggen-ui.umd.js` as a thin iframe + postMessage wrapper
5. **Vite build**: Add embed entry to main app build (or separate small bundle)

## Open Questions

1. **Backward compatibility**: Should we keep the old React SDK as `LinggenUI.mountLegacy()` for existing skills?
2. **Dark/light theme**: How does the iframe respect the parent page's theme? Via query param `?theme=dark`.
3. **Height auto-resize**: Should the iframe auto-resize based on content? Or always fill container?
4. **Offline/dev mode**: When running `npm run dev` (Vite), the embed route needs to work too.
