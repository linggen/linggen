# UI SDK — Build Interactive Skills with Chat

The Linggen UI SDK (`@linggen/ui-sdk`) is a drop-in chat component for building interactive app skills. It handles session management, SSE streaming, markdown rendering, and thinking indicators — so your skill only needs to focus on its own UI and game logic.

## Quick Start

The fastest way to build a skill is to ask Linggen itself:

```
ling "Create a new interactive app skill called my-app under ~/.linggen/skills/my-app.
It should have a web UI with a chat panel using the Linggen UI SDK.
See the game-table skill as a reference."
```

Linggen will scaffold the skill directory, write the `SKILL.md`, HTML, CSS, and JS files for you.

## Skill Structure

```
~/.linggen/skills/my-app/
├── SKILL.md              # Skill definition (required)
└── scripts/
    ├── index.html         # Entry page
    ├── style.css          # Shared styles
    ├── app.html           # App page with chat
    ├── app.css            # App-specific styles
    └── app.js             # App logic
```

## SKILL.md Configuration

The `app` section in frontmatter makes your skill a launchable web app:

```yaml
---
name: my-app
description: My interactive app with AI chat
allowed-tools: []
app:
  launcher: web
  entry: scripts/index.html
  width: 900
  height: 700
---

You are an assistant for my-app. Respond to user messages concisely.
```

Key fields:

| Field | Purpose |
|:------|:--------|
| `name` | Must match the directory name |
| `description` | Shown in skill list; helps the model decide when to use it |
| `allowed-tools` | Tools the agent can use in this skill's sessions (`[]` = chat only) |
| `app.launcher` | `web` — serve as static files in an iframe |
| `app.entry` | Path to the entry HTML file |
| `app.width/height` | Suggested window size in pixels |

The markdown body below the frontmatter is injected into the agent's system prompt for every message in a skill-bound session.

## Using the UI SDK

### 1. Load the SDK

Add a script tag in your HTML — no npm install or build tools needed:

```html
<script src="/sdk/linggen-ui.umd.js"></script>
```

The SDK is served by Linggen at `/sdk/`. It bundles React and all dependencies — zero external requirements.

### 2. Add a Chat Container

```html
<div id="chat-panel"></div>
```

### 3. Mount the Chat Panel

```javascript
const chat = LinggenUI.mount(document.getElementById('chat-panel'), {
  skillName: 'my-app',
  agentId: 'ling',
  modelId: selectedModelId,        // from /api/models, or omit for server default
  title: 'Chat',
  placeholder: 'Type a message...',
  onSessionCreated: (sessionId) => {
    // persist session ID in URL, localStorage, etc.
  },
  onMessage: (message) => {
    // called when a complete AI message is received
  },
  onStreamToken: (fullText) => {
    // called on each streaming token with accumulated text
  },
  onStreamEnd: (text) => {
    // called when streaming completes with final text
    // use this to parse structured responses like [MOVE] tags
  },
});
```

### 4. Use the Chat Instance

```javascript
// Send a message (creates session, calls API, streams response)
chat.send('Hello!');

// Send a hidden message (not shown in chat UI)
chat.send('[BOARD_MOVE] some structured data');

// Add a display-only message (no API call)
chat.addMessage('ai', 'Welcome to the app!');
chat.addMessage('system', 'Game started.');

// Clear all messages
chat.clear();

// Get the session ID
const sid = chat.getSessionId();

// Clean up when done
chat.destroy();
```

## ChatPanelOptions Reference

| Option | Type | Default | Description |
|:-------|:-----|:--------|:------------|
| `serverUrl` | `string` | `''` | Base URL for Linggen server (same-origin by default) |
| `skillName` | `string` | — | Skill name to bind sessions to |
| `agentId` | `string` | `'ling'` | Agent ID |
| `sessionId` | `string` | — | Reuse an existing session instead of creating one |
| `modelId` | `string` | — | Model ID; omit to use server default |
| `title` | `string` | — | Header text above the chat |
| `placeholder` | `string` | — | Input placeholder text |
| `className` | `string` | — | Additional CSS class on the root element |
| `onSessionCreated` | `(id) => void` | — | Called when a new session is created |
| `onMessage` | `(msg) => void` | — | Called with complete messages |
| `onStreamToken` | `(text) => void` | — | Called on each token with accumulated text |
| `onStreamEnd` | `(text) => void` | — | Called when streaming finishes |

## ChatInstance Methods

| Method | Description |
|:-------|:------------|
| `send(text)` | Send a message via API, triggering AI response |
| `addMessage(role, text)` | Add a display-only message (`'user'`, `'ai'`, or `'system'`) |
| `clear()` | Clear all displayed messages |
| `destroy()` | Unmount React root and clean up SSE connection |
| `getSessionId()` | Returns current session ID or `null` |
| `setOptions(opts)` | Update options dynamically |

## Patterns

### Hidden Messages (Board Moves)

Send structured data to the agent without showing it in the chat. Messages starting with `[BOARD_MOVE]` are hidden from the chat UI:

```javascript
chat.send('[BOARD_MOVE] game state and move data here');
```

Use `onStreamEnd` to parse structured responses:

```javascript
onStreamEnd: (text) => {
  const match = text.match(/\[MOVE\](.*?)\[\/MOVE\]/);
  if (match) {
    const move = JSON.parse(match[1]);
    applyMove(move);
  }
}
```

### Model Picker

Fetch available models and let users choose:

```javascript
const res = await fetch('/api/models');
const models = await res.json();
// populate a <select> element, pass chosen model as modelId
```

### Session Persistence

Store the session ID in the URL so users can resume:

```javascript
onSessionCreated: (sid) => {
  const url = new URL(location.href);
  url.searchParams.set('session', sid);
  history.replaceState({}, '', url);
}

// On load, check for existing session:
const sid = new URL(location.href).searchParams.get('session');
LinggenUI.mount(container, { sessionId: sid, ... });
```

## Example: game-table

The `game-table` skill is the reference implementation. It demonstrates:

- Lobby page with game selection (`index.html` + `main.js`)
- Game pages with board + chat side-by-side (`xiangqi.html`, `gomoku.html`)
- Hidden board moves via `[BOARD_MOVE]` prefix
- Parsing `[MOVE]` tags from AI responses in `onStreamEnd`
- Model switching, session persistence, score tracking

Location: `~/.linggen/skills/game-table/` (or `skills/game-table/` in the source repo).

## Styling

The SDK injects its own CSS automatically. The chat panel renders inside a `.lc-root` container. To control its size, style the container element:

```css
#chat-panel {
  width: 360px;
  height: 100%;
  flex-shrink: 0;
  overflow: hidden;
}
```

The SDK respects CSS custom properties from the host page. If your skill uses the shared `style.css`, the chat will match the app's theme automatically.

## API Endpoints Used by the SDK

The SDK calls these Linggen server endpoints (same-origin, no CORS config needed):

| Endpoint | Purpose |
|:---------|:--------|
| `GET /api/models` | Fetch default model |
| `POST /api/sessions` | Create a skill-bound session |
| `POST /api/run` | Send a chat message |
| `GET /api/events?session_id=` | SSE stream for responses |
