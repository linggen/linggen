This extension lives in the Linggen repo: [github.com/linggen/linggen/tree/main/browser-extension](https://github.com/linggen/linggen/tree/main/browser-extension).

# linggen-browser

The Linggen browser bridge — a Chrome (MV3) extension connecting your browser
to a local Linggen daemon:

- **Session reads** (X first): skills read your *logged-in* sessions on demand,
  read-only, without paid APIs. Cookies never leave the browser; only parsed
  results cross the bridge.
- **Browser control**: a Linggen agent can operate one **visible** tab via ten
  ops — `navigate`, `read_page` (accessibility tree with element refs),
  `click`, `type`, `key`, `scroll`, `screenshot`, `wait`, `tabs`,
  `read_console` — with per-action confirmation until you trust a site for the
  session, and sensitive actions (payment, passwords, deletes, posting as you)
  always confirmed. Control uses `chrome.debugger` (CDP), so Chrome shows its
  "started debugging this browser" banner while the agent works — expected,
  not malware.

It is a **global** bridge: a host shell plus **modules** (`x`, `control`).
More sites are added as modules without a new Web Store listing.

Protocol contracts: `doc/browser-bridge-spec.md` and
`doc/browser-control-spec.md` (the daemon side lives in this repo).

## How it works

- The background worker holds a WebSocket to the daemon at
  `ws://127.0.0.1:9527/api/bridge/socket`.
- The daemon pushes a `req` frame; the extension runs the read and answers with
  a `res` frame. Reads are on-demand — nothing is harvested in the background.
- For a read, the extension opens a background `x.com` tab at the relevant URL
  (e.g. `/search?q=…`). A `document_start` hook (`x-hook.js`, MAIN world) wraps
  `fetch`/XHR and forwards X's **own** GraphQL responses; an isolated relay
  passes them to the worker, which normalizes the result and closes the tab.
  Because the *page* issues and signs the request, the extension forges nothing
  — X rotating its `queryId` / `features` / `x-client-transaction-id` can't
  break it.

## Layout

```
manifest.json
src/
  background.js        bridge transport: socket, handshake, req→module dispatch
  cdp.js               tab-scoped CDP helpers: attach, evaluate, human-scale input
  modules/x.js         X module: on-demand tab + response capture → normalize
  modules/control.js   control module: visible controlled tab driven via CDP (chrome.debugger)
  content/x-hook.js    MAIN-world fetch/XHR hook (document_start): forwards X's graphql responses
  content/x-relay.js   isolated-world relay: page hook → service worker
  popup/               connection-status popup
```

## Develop

Load unpacked: `chrome://extensions` → Developer mode → Load unpacked → this
folder. The status popup shows whether the daemon socket is connected. After
editing any file, hit the reload icon on the extension card; the socket
reconnects within a minute (keepalive alarm).

Works in Chrome and Chromium forks — verified in Arc.

## Status

Transport, protocol, on-demand tab, and response interception are complete.

- **x module** (v17): `search`, `targets`, `following`, `whotofollow`, `own`
  (own posts + already-replied parents), and `mentions` (the Mentions
  notifications tab, shaped into reply_to_me / mention items) are wired;
  `user_lookup` / `followers` follow the same intercept pattern and are not
  wired yet. `post` is the one write op — a visible tab, typed and clicked
  through CDP, behind the gate's always-confirm floor, and resolved only on
  x.com's own `CreateTweet` answer.
- **control module** (v1): all ten ops implemented and verified end-to-end
  against the daemon (`Browser_*` engine tools on the Linggen side).

Distribution: Chrome Web Store listing pending; until then, load unpacked.

## License

Apache 2.0, the same as the rest of this repository — see `LICENSE`.
