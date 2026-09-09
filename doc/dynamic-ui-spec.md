---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
status: designed 2026-09-09; step 1 built the same day on the phone (screen registry, one door, layout contract, Yinyue's four screen tools, Health Highlights as the first screen); the signals and the notice are designed, not built
---

# Dynamic UI

A screen is a document. The app renders the document; the data layer fills
its numbers; Yinyue rearranges it by writing the document back within a
schema that says what may move; and every time it moves, she says so in her
thread. Apple shows one screen to everyone. Ours is composed for the person,
and the person can always see who composed it, why, and put it back.

## Rules

1. **The screen is JSON, rendered.** What a screen shows is a document on the
   phone — sections in order, each with a kind and a payload. The renderer
   draws documents; it holds no opinion about which document. A section kind
   the renderer cannot draw is refused, whole, before it reaches the screen.
2. **The schema says what may move.** Each screen publishes its schema: the
   sections it has, in order, each `fixed` or `adjustable`. A fixed section
   is composed by rules from the data and nobody rearranges it — a warning, a
   brief, a finding. An adjustable section carries a catalog of everything it
   could show today and the hands it takes (select, pin, unpin, hide, unhide,
   kind, undo). The schema is the contract; the renderer, the rules, Yinyue
   and the Mac all read the same one.
3. **Values are the data layer's.** The catalog is computed from the store on
   this phone. Yinyue chooses among catalog entries by id and never supplies
   a number, a label or a chart. The validator refuses a document that names
   what the catalog does not hold. This is what makes a model-authored screen
   safe to show.
4. **The person's hand beats hers.** A pin holds; a hide holds; a choice made
   today holds today. Yinyue's arrangement is refused where it would cross
   one of those, and the refusal is final for that turn. User actions are
   never gated; hers are.
5. **She moves on signals, not on a clock.** The document recomposes on: the
   first read of the data, data that changed enough to change the catalog,
   a question the person asked (asking stamps the subject), and the night's
   pass. Each of those is a notice to Yinyue with what changed and why. She
   may then arrange further, or leave it.
6. **She tells them, once, in her thread.** When the screen moved — by rules
   or by her — she says it in one line in the conversation, never on the
   view: what is on top now, in their terms, why, and that they can ask for
   it another way. A recomposition that produced the same screen says
   nothing. One line per screen per day at most.
7. **Undo is one tap, and it pins.** Every document records the one it
   replaced. Undo puts it back and pins its cards for four weeks, so the
   same move is not proposed again next week.
8. **The Mac reads the same document.** A paired Mac shows the same screen
   from the same file and may change the selection through the same
   validator. It never recomputes a value.

## The document

One file per screen (`layout.json` for Health Highlights today). It carries:

- `composed_at`, `pass` (what composed it: night, goal, asked, user, agent,
  undo), `by` (rules or model), `why` in the person's terms, `previous`.
- `sections`, in the schema's order. A fixed section carries what the rules
  composed. An adjustable section carries its `catalog` (id, subject,
  question, title, kind, kinds, period — never values in the contract the
  agent reads; the values ride the same entry for the renderer), what it
  draws now (`cards`, `selected`, `selected_by`, `why`), the person's hands
  (`pinned`, `hidden`, `kinds`, `asked`), and its `hands`.

Health Highlights is the first document: Brief fixed, Focus adjustable,
Attention fixed. Focus draws a short stack — what the person pinned or
chose, then what they asked about this week newest first, then the rules'
pick — at most four, a week's hold on an ask, never a card the catalog
cannot draw.

## The registry and the door

Every screen declares itself: its sections, and its pages below them with a
line about each. The shell folds them into one registry of routes — `health`,
`health/everything`, `health/plan` — and one door that opens any of them. A
notification tap, a link on a tool chip in the thread, and Yinyue's
`open_screen` all go through that door, so nothing listed can fail to open
and nothing unlisted can be opened by name.

## Yinyue's hands

Four tools, one per verb:

| Tool | Does |
| --- | --- |
| `screens` | every route, with a line each |
| `open_screen {route}` | open one |
| `screen_layout {route}` | the screen's document under its schema: sections fixed or adjustable, the catalog and hands of an adjustable one, what it draws now and who set it |
| `screen_arrange {route, action, id, kind, why}` | a hand on an adjustable section, through the screen's own validator; a fixed section is refused |

A screen with nothing adjustable still answers `screen_layout` with its fixed
sections, so she can describe what the person is looking at.

## The signals, and the notice

| Signal | What recomposes | What she is told |
| --- | --- | --- |
| first read of the data | the whole document | "Your Health page is composed: X on top because Y." |
| data changed enough to change the catalog | the adjustable sections | what entered or left the catalog |
| the person asked about a subject | Focus gains its card | "you asked about steps" |
| the night's pass | the whole document | what moved, and the finding behind it |
| the person's own hand | nothing recomposes; the hand is kept | nothing — they did it |

On each, she reads the contract, may arrange further with a `why`, and says
one line: *"I put steps and sleep on top of your Health page, since you
asked about them. Say the word if you want it another way."* Silence when
nothing moved. The line is hers, in the thread; the view stays quiet. Her
reply to "put sleep back on top" goes through `screen_arrange`, and the
thread shows the door to the screen.

## What must never happen

- A number, label or chart authored by the model on a screen.
- A section moving under the reader: a routine recomposition waits behind
  an *Updated* chip; only the person's own hand shows at once.
- A screen changed with no line in the thread saying so.
- A fixed section rearranged by anyone.
- A dead knob: a hand the schema lists that no section takes.

## Not yet

Other screens' adjustable parts, when one earns it — a screen with none is
fine; the DJ, CFO and Shifu page registries; the signal-to-notice wiring and
the one-line notice; the Mac reading the contract rather than only the
selection.

## Where the detail lives

Health's document and validator: `linggen-mobile/lib/services/health/health_home.dart`
and the Health design doc in the skills repo. The registry and the door:
`linggen-mobile/lib/main.dart`. Her tools: `linggen-mobile/lib/services/yinyue/yinyue_tools.dart`.
The phone-side manual: `linggen-mobile/doc/yinyue.md` (The screens, and
what she may arrange).
