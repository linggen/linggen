---
type: spec
reader: Coding agent
guide: |
  The wire contract for Mac → phone push. Keep it brief; the code carries the
  details. Three repos build against this text: linggen (engine), linggensite
  (relay), linggen-mobile (phone).
---

# Phone push — the doorbell and the sealed note

Built 2026-10-27. When a line the Mac's missions found must reach a phone that
is closed, the Mac rings APNs through a relay on linggen.dev. The Mac decides,
the Mac's Yinyue writes the words, the Mac seals them for that one phone. The
relay only checks who is asking and forwards. Data still rides WebRTC; push
carries one sealed line and nothing else.

```
skill declares a line ─► mission ends / app raises ─► engine queue (persisted)
   ─► due, not told, line on, budget left ─► her words (one model call)
   ─► AES-256-GCM sealed with the phone's key ─► POST linggen.dev/api/push/send
   ─► relay: Mac's account owns that token? ─► APNs (HTTP/2, ES256 JWT)
   ─► phone: Notification Service Extension opens it, or shows the fallback
```

## Who holds what

| Holder | Holds |
| --- | --- |
| Phone | its APNs token, a 32-byte push key (Keychain, shared with the extension through the app group), its told list |
| Mac | per paired phone: token, environment, key, language, fallback line, morning hour, lines switched on, what the phone has told |
| Relay | token → account (KV), the `.p8` as `APNS_KEY`. Never a key, never plaintext |

The push key is made on the phone and handed to its Mac over the paired
WebRTC channel — the channel pairing set up, so the key never leaves the two
devices. A phone paired before push existed hands it over on its next connect.

## Phone → Mac (WebRTC `http_request`, actor-stamped)

- `POST /api/push/device` — `{token, env: "sandbox"|"production", key: base64(32 bytes), language, fallback, morning: "HH:MM", lines: {"<app>.<line>": bool}}`. Sent on every connect (identify), so a switch flipped on the phone reaches the Mac next time it can. `env` is read from the signed profile's `aps-environment`, never from the build configuration.
- `DELETE /api/push/device` — push off for this phone.
- `POST /api/push/told` — `{told: ["<app>.<line>:<key>", …]}`: lines the phone said or scheduled itself, or a push already told. The Mac never pushes a told line.
- `GET /api/push/status` — what the settings line shows: registered or not, the last push and how it went.

## Declaring and raising a line (any skill)

A skill declares its lines in `SKILL.md`, beside its tools:

```yaml
notices:
  - line: watch                 # herald id <skill>.<line>
    tier: record                # warning | hand | record
    prompt: notices/watch.md    # her job for this line; the phone's prompt shape
    after: watch                # optional: raised when this skill's mission ends ok
    facts: perl scripts/market.pl watch-notice   # optional: prints the notice
```

`facts` prints one JSON object — `{key, facts, route?, until?}` — or nothing,
which means there is nothing to say. Code may also raise a declared line at
any time: `POST /api/push/notice {app, line, key, facts, route?, until?}`
(loopback only). The engine names no app; it carries the declaration.

## When the Mac pushes

- **warning / hand** — two minutes after it is raised. A phone that is
  connected picks the line up live, acks it, and the push never goes.
- **record** (the morning line) — at the phone's morning hour when the line
  was raised before it, else thirty minutes after it was raised. The phone's
  02:00 wake usually takes the brief and schedules its own morning line; its
  ack stops the push. The push is the case where the wake never came.
- Never after `until` (default: noon for a record line raised before noon,
  else six hours). `apns-expiration` is set from it.
- Two lines a day per phone, a warning never counted.
- Her words come from one model call: her identity (the same `agents/yinyue.md`
  the phone reads), the skill's prompt, the facts as data, in the phone's
  language. Reply `{"title", "text"}`. No reply, no push — it waits for the
  next tick until `until`.

## The sealed note

Plaintext: `{v: 1, app, line, key, told: "<app>.<line>:<key>", title, text, route, tier, at}`.
Sealed: AES-256-GCM, 12-byte random nonce, AAD `linggen-push-v1`, sent as
base64 of `nonce ‖ ciphertext ‖ tag` (CryptoKit `AES.GCM.SealedBox(combined:)`).
At most 3000 base64 characters.

## Mac → relay

`POST https://linggen.dev/api/push/send`, `Authorization: Bearer <the Mac's account token>`:

```json
{"token": "<hex>", "sealed": "<base64>", "collapse_id": "<16 hex>",
 "priority": 5, "expires_at": 1793000000, "fallback": "银月有话说"}
```

`priority` 10 for warning and hand, 5 for record. `collapse_id` is a hash of
the told key, so it names nothing. Answers:

| Status | `code` | Mac does |
| --- | --- | --- |
| 200 | — (`apns_id`) | marks told |
| 401 | `unauthorized` | waits (signed out) |
| 404 | `not_registered` | waits; the phone re-registers on its next connect |
| 410 | `gone` | drops the token (APNs said Unregistered, or the phone was unpaired) |
| 429 | `rate_limited` | waits |
| 503 | `not_configured` | waits (no `.p8` yet) |
| 502 | `apns_error` (`reason`) | waits, logs the reason |

## Phone → relay

`POST /api/push/register {token, env, instance?}` and `DELETE` the same,
authorized by the phone's relay grant for its Mac (`instance` = that Mac's
id) or by an account token signed in on the phone. The relay keeps
`token → {account, env, grant}` in KV; a send checks the grant still exists,
so unpairing a phone stops its pushes too.

## The relay's payload to APNs

`{"aps": {"alert": {"body": fallback}, "mutable-content": 1}, "s": sealed}`,
`apns-push-type: alert`, topic `dev.linggen.linggenMobile`. The host follows
the registered environment; a `BadDeviceToken` tries the other host once
(a TestFlight build vs a development one) and re-registers to it on success.
JWT ES256, cached 40 minutes. Rate: 30 an hour, 200 a day per account.

## On the phone

The Notification Service Extension opens the note with the key, sets title,
body, thread (the app), route, and the level from the tier — record passive,
hand active with sound. It writes `{told, title, text, route, at}` to the app
group (`push/told.json`). It cannot suppress: when opening fails the fallback
line shows, nothing private. The app reads that file before telling a line
itself: a line a push told goes to her thread in the push's words and is not
said again; and every line the app tells or schedules is acked to the Mac.
