---
type: spec
reader: Coding agent and users
status: draft 2026-10-04, awaiting go
guide: |
  Product specification — what the system should do and why. Brief; guides
  design and implementation, not a code walk-through.
---

# Shared sessions

One session can hold more than one agent. Ling and Yinyue sit at the same
table, see the same thread, use the same tools, and answer under the same
permissions; each keeps her or his own persona and model. This replaces
guest mode.

## Why

- Guest mode made Yinyue a visitor: she saw a trimmed view (dialogue only,
  last lines, no tool results, no recall), so in Lingjing she did not know
  the plot and had to be told it.
- Guest turns run beside the host's turn, with their own permissions and a
  special engine path. Every surface (restore, compaction, export, UI
  filter, moments) carries a guest branch.

## Model

| Belongs to the session | Belongs to each agent |
|---|---|
| The thread (`messages.jsonl`) and its compaction | Persona (system prompt) |
| Tool results and recall notes in that thread | Model |
| Tool set | |
| Permissions (`permission.json`) | |
| Turn lock — one agent speaks at a time | |

A session declares its members:

```
agents: [{id: "ling", model: "gpt-5.6-sol"}, {id: "yinyue", model: "gpt-5.6-luna"}]
```

- A member's model defaults through today's chain (session override →
  agent config → routing default; Yinyue's `pet.model`).
- Defaults: an ordinary chat is `[ling]`; Yinyue's daily thread is
  `[yinyue]`; a skill declares its members in frontmatter (Lingjing:
  `[ling, yinyue]`).
- `@name` in a message addresses that agent and, if absent, adds her or him
  to the session for good. Absence rules (`absent_until`) still gate who is
  present.
- Her daily sessions stay as they are (`sess-yinyue-YYYY-MM-DD`, listed in
  All as today).

## Who answers

- An addressed message (`@name`) goes to that agent.
- Otherwise the surface's default answers: **Ling on the Mac, Yinyue on the
  phone**, if a member; else the session's first member.
- One agent per turn. The other reads the turn when its own comes.
- Skill-declared moments (a story node in Lingjing) may wake a named member;
  that is a turn like any other.

## The thread

The session thread is the one source of truth. Each turn, the speaking
agent's context is built from it:

- Its own messages → assistant turns. Another member's messages → labelled
  dialogue (`[Yinyue]: …`), never its own words.
- Its own tool calls → native call/result pairs. **Another member's tool
  calls → labelled text** (`[Ling used Look → …]`), never native pairs —
  providers reject or mis-handle calls they did not make (Anthropic with no
  tools defined, Gemini thought signatures).
- Recall → persisted rows, shown to every member as system notes. Whose
  recall policy applies: the session's (owner chat, skill scope).
- Moments and heralds land as rows in the session, not side channels.

## Compaction

- One compaction for the whole thread, saved as one summary row every
  member reads.
- Trigger: the **smallest member's** context window × the session's
  threshold, so no member overflows on its turn. A member's window is
  re-read when its model changes.
- The summarizing model is the session's default member's.

## Tools and permissions

- The tool set belongs to the session, set by what created it: an ordinary
  chat gets Ling's set; Yinyue's daily thread gets hers; a skill session gets
  its `allowed-tools`. A member who joins later uses the session's set.
- Permissions are the session's: read by default; writes and Bash ask the
  person, whichever member asks. Guest seat permissions go away.
- Style stays in the persona: `yinyue.md` says code and file work is Ling's,
  and in a shared session she hands it to him at the table.

## What changes

1. Thread on the session: build every member's context from
   `messages.jsonl` as above; fix today's role bugs (compaction and recall
   rows replayed as assistant turns).
2. Session turn lock, replacing the guest queue.
3. `agents` on session meta; engines per (session, agent) or rebuilt per
   turn; per-member model.
4. Shared, saved compaction with the smallest-window trigger.
5. Session tool set and permissions.
6. Remove guest mode: the guest engine path, seat permissions, side lines,
   the guest branch in export, restore and UI. Moments land as rows.
7. UI: the agent picker becomes the session's member list; the message
   filter shows every member; export previews per member.

## Open

- **Phone.** The phone's Yinyue is a local LLM loop with its own thread and
  phone tools, not a Mac session. "Yinyue answers by default on the phone"
  for a Mac shared session would run a second, Mac-side Yinyue. How the two
  relate is decided after the Mac side lands.
- **Speed.** Yinyue in a long shared thread reads more than her 10-message
  home window; her replies slow down. Watch it in Lingjing before tuning.
- **Tool widening.** Several 2026-09 incidents were fixed by narrowing her
  tools (AskUser hang, Skill pickup, agent_chat loop). Re-test each in a
  shared Lingjing session before removing the narrowing tests.
