---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Rooms

> **Off by default.** Rooms are not what the product is for right now, so the
> whole feature sits behind one switch that is off unless `room_config.toml`
> says `rooms_on = true` (read once at start). Off, every way into rooms
> answers "rooms are off", the pages show nothing about them, and room records
> are left untouched. The code stays as written below and returns with online
> play in 灵境. The switch lives in `src/server/rooms_switch.rs`.

Community model sharing. An owner opens a **room** to share their models with other users (consumers); the owner's linggen acts as the proxy that calls the AI provider. Consumers get AI access without setting up keys; owners share spare capacity. Built on linggen's existing WebRTC infrastructure.

## Related docs

- `webrtc-spec.md` — transport, signaling, data channels.
- `permission-spec.md` — permission modes, room ceiling, consumer enforcement.
- `session-spec.md` — session lifecycle.

## Motivation

New linggen users face a cold-start problem: they need API keys before they can do anything. Keys cost money and require signup with providers. This friction kills adoption.

Meanwhile, existing users often have unused capacity — paid subscriptions with idle tokens, local Ollama models sitting quiet. There's no way to share this surplus.

Rooms solve both problems. Owners share spare capacity through their linggen. New users can chat without API keys. The experience feels like a cloud service, but it's powered by the community.

### Goals

- Zero-setup onboarding: new users chat immediately via community rooms.
- Reward owners with credits (not money) — grows the ecosystem.
- Two trust levels: public rooms (anyone) and private rooms (invited users).
- Auto-dispatch: users can be matched to available rooms automatically.
- Star topology: one owner, multiple consumers — no mesh complexity.
- One owned room per user: prevents proxy loops and simplifies the mental model.

### Non-goals

- Payment processing or real money exchange.
- End-to-end encryption (the owner must see messages to call the AI provider).
- Mesh topology between consumers.
- Guaranteed uptime or SLA.

## Membership

- **One room per owner.** A user can own at most one room. To create a new room, delete the existing one.
- **A user can join multiple rooms as a consumer**, but cannot join their own room.
- **No re-sharing.** Only the owner's local models (Ollama, OpenAI, etc.) can be shared. Models received from other rooms are never eligible for sharing — prevents proxy loops.
- **Owners can remove any member**; consumers can leave any room. Deleting a room removes all members.

## Topology

Each room has exactly one owner and up to a small number of consumers (default 4). Star — all consumers connect to the owner via WebRTC data channels; consumers do not connect to each other. From the owner's linggen, each consumer is just another peer with its own session.

## Room types

### Public rooms

Visible on linggen.dev's room browser. Anyone with a linggen account can join (costs credits). The owner accepts that any community member may use their models. Public rooms back the "try linggen for free" experience and feed auto-dispatch.

### Private rooms

Invite-only. The owner generates invite links or adds specific users. Not listed in the browser. For family, teams, or friends — the owner knows who's using their models.

## Discovery

### Manual browse

linggen.dev lists public rooms with: available models, open slots, region/latency, remaining token budget, owner uptime history.

### Auto-dispatch

User clicks "Start chatting" on linggen.dev — no room selection needed. The dispatcher filters online public rooms with available slots and budget, ranks by preferred model and load, and connects to the best one. From the consumer's perspective, it looks like a cloud service.

## Consumer modes

### Browser mode

Consumer opens linggen.dev. The owner's linggen UI is tunneled through the data channel — consumer gets the full chat experience (markdown, tools, skills) running in their browser, restricted by the owner's room settings.

A dedicated consumer chat surface shows only consumer-relevant UI: chat, session list, allowed skills, room name, privacy warning, token budget, leave button. Owner-only features (settings, file browser, project selector, mission editor) are hidden.

### Linggen-server mode

Consumer runs their own linggen and uses the owner's linggen as a remote model provider. The consumer's agent runs locally with all its tools; only inference is proxied. The consumer's files stay local — the owner only sees inference requests.

Proxy models appear in the consumer's model selector with the room name attached, e.g. `gpt-5.4 (Tom's Room)`.

## Owner controls

The owner configures their room from Settings → Sharing:

- **Shared models** — which configured models consumers can use.
- **Allowed tools** — preset modes (chat / read / edit) plus individual checkboxes.
- **Allowed skills** — which skills consumers can trigger.
- **Max consumers** — concurrent user cap.
- **Token budgets** — daily caps, room-wide and per-consumer.
- **Room type** — public or private.
- **Members** (private rooms) — invite link, member list with remove.

Disabling a room broadcasts a kick to current consumers and updates the linggen.dev directory.

## Permissions

The room config is the **ceiling** for what a consumer can do. Even a consumer with edit permission cannot use tools the owner hasn't shared. Consumer sessions never show permission prompts — all enforcement is server-side. See `permission-spec.md` for the full model.

## Token budget

The owner sets a daily budget per room and an optional per-consumer cap. When a budget is exhausted, the room appears "full" — no new sessions, existing sessions finish their current turn. Budgets reset at midnight UTC. Enforcement is on the owner's linggen (it controls the API calls); usage is also reported to linggen.dev for credit accounting.

## Privacy and trust

The owner's linggen processes all messages — it must, in order to make API calls. This is stated clearly to consumers before joining.

| Room type | Trust level | Privacy expectation |
| :-------- | :---------- | :------------------ |
| Private   | High (invited users) | Family/team — trusted context |
| Public    | Low (strangers) | Use at own risk — don't share secrets |

Mitigations: clear warning before joining a public room; community conduct policy on room creation; abuse reporting and de-listing; session history not persisted on the owner by default for room consumers (configurable).

## Provider considerations

Rooms use the owner's API keys. This is technically sharing API access, which some providers restrict. Linggen's position:

- Credits are not money — community sharing, not reselling.
- Closed rooms (family/team) are clearly within reasonable use.
- Public rooms are positioned as community generosity, not a service.
- Local models (Ollama) have no TOS concerns and are ideal for public rooms.
- If a provider objects, their models can be excluded from public rooms.

## Credits — TODO

> Not yet implemented. Tracked as a future phase.

Credits are the planned community currency — not real money, not transferable, not purchasable.

- **Earning** — owners earn credits proportional to tokens served (input + output). Only successful completions count.
- **Spending** — consumers spend credits at the same rate when using rooms. When the balance hits zero, the session pauses with guidance.
- **Starter credits** — new accounts receive a small allowance to try the service.
- **Other unlocks** — higher instance limits, priority in auto-dispatch, profile badges.

What credits are not: not cashable, not transferable, not purchasable, no marketplace.
