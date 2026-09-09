---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
status: designed and built 2026-09-08 (ling-mem 1.8.0, engine 6e331e9, phone); per-person folders on the phone 2026-09-09; the dream's per-account iteration on the Mac is the one open piece
---

# Phone Memory

What Yinyue remembers on the phone, how it stays short, and how it meets the
Mac's memory when there is one. One logic for a phone-only user and a paired
one: the Mac, when present, is a better judge, not a different design.

Before this, the phone kept one markdown file with an append-only Notes
section that rode whole in every prompt, and a phone without a Mac never got a
core at all. Both are what this spec replaced.

## Rules

1. **Only Yinyue writes.** She is the app. DJ, CFO, Health and Shifu are her
   senses and hands; they expose facts through tools and the activity log, and
   she decides what is worth keeping. One writer, one voice in the file.
2. **Facts, not events.** Every note is born long-term and tagged with the app
   it serves. Events live in the activity log (three days) and reach the Mac's
   dream as a day digest. The phone holds no short-term tier.
3. **Few notes.** She writes what will still be true next month: who they are,
   the music they reach for, the budgets they watch, the conditions and goals
   they are working on, the devices they own. The dream throws away; she only
   appends.
4. **The Mac's memory is read-only on the phone.** The phone maintains its own
   rows and hands them up. It never edits, promotes or deletes a Mac row by
   itself; the user may, from the phone, because user actions are never gated.
5. **Every row names its person.** Two phones with two accounts can pair to
   one Mac; a row from a phone carries the account it came from, stamped by the
   Mac from the pairing record, never asserted by the phone.

## Tiers on the phone

- **About them** — the core tier: name, language, place, family, routines.
- **App facts** — long-term rows, one block per app: `dj`, `cfo`, `photos`,
  `health`, `shifu`, `yinyue`. This is the app tag on the row; the same field
  the Mac's store already uses for scope, so nothing new is invented.

All rows ride in every prompt, About them first, then one block per app, so
the DJ facts sit together when she is picking music. No search on the phone.
When a Mac is reachable, her deeper lookup searches the Mac's full store.

## Cap

1000 rows. Nobody normally reaches it. When an add would cross it, the ten
oldest long-term rows go first, oldest by last touch: a row the dream merged or
confirmed was touched, a row nobody has needed was not. Core is never evicted.
With a Mac paired the evicted row is still on the Mac and returns through the
pull if the Mac keeps it; for a phone-only user it is gone, the accepted price
of a cap.

## The nightly pass

Runs on the phone's own model, the one Yinyue uses, chosen in Settings. Rides
the app's one background wake, the same wake Health already uses. Housekeeping
only, since there is nothing to promote from a short-term tier: merge
near-duplicates, lift a fact about the person into About them, expire a row a
newer one contradicts, keep the store under the cap. One doctrine with the
Mac's dream, minus its short-term step.

Then, if a Mac is reachable: push, reap, then pull, in that order, so a note
taken offline reaches the Mac before the phone asks what the Mac knows, and a
delete made while the Mac was away reaches it before the pull could undo it.

The pass's runbook ships inside the app rather than being fetched: a
phone-only user has no Mac to serve it from. It is the Mac's dream doctrine
minus the short-term step, and changes to one should be made to the other.

## With a Mac

**Up.** Each note goes up as short-term, however it is held on the phone. The
Mac's dream decides whether it lasts and in which tier. Exact duplicates are
refused by the store at add time; near-duplicates are the dream's job on both
sides, never a phone-side search before every note. The add's reply carries
the Mac's row id, and the phone keeps it on its copy.

**Down.** The pull returns everything the Mac holds for this account that this
phone's apps can act on: the core tier, long-term rows tagged with a phone
app, and the ids of rows still staged short-term. This is why the app tag
matters: "I like 90s Hong Kong songs" is a DJ fact, not a core one, and it
must come back. The dream carries the tag through promotion, so a note tagged
on the phone stays tagged on the Mac. The staged ids are what let the phone
tell "not judged yet" from "dropped".

**Who wins.** A phone row with a Mac id follows the Mac: the pull's version
replaces it, and a row the Mac dropped is dropped on the phone. A row without a
Mac id is the phone's own, until it lands. Nothing vanishes between landing and
the Mac's dream: the phone keeps its copy until the Mac's answer replaces it.

**Sync moments.** When her chat screen opens, when the route to the Mac comes
back, after each turn she took a note in, and the nightly pass.

## Accounts

- The store gains the account id and name on every row. Rows without one are
  the Mac owner's; nothing existing needs migrating.
- The engine's memory door stamps them from the paired device's account, on
  every verb. The phone never sends them and cannot claim another account.
- Recall, list, delete and the dream all see one account at a time.
- A signed-out phone writes under its device id. At the first connect after
  sign-in, those rows are re-stamped to the account, once.
- The account is what the phone told the Mac at pairing and on each connect,
  and again the moment it changes on a live channel. It is a household ledger,
  not a security boundary.

### On the phone (2026-09-09)

The same rule holds on the phone: memory is kept per person. Under
`yinyue/` there is one folder per signed-in account, named by its id, and a
`device` folder for the stretches when nobody is. Her rows, the outbox and the
tombstones live in that folder; `device.md`, where she is running, stays at
the root because it is the phone's, not a person's.

- Switching accounts switches what she remembers and what goes up. Sign-out
  moves her to the device folder and leaves the account's folder on disk, out
  of her prompt, for when that person returns.
- Sign-in and sign-out are sync moments: the transport re-introduces the phone
  to the Mac at once, so the Mac stamps the right person from then on, and the
  sync pushes, reaps and pulls for the new person.
- The device folder is adopted once: the first account to sign in after it was
  written takes its rows and its queued notes, and the folder is left empty
  for the next signed-out stretch. This is the phone's half of the Mac's
  one-time re-stamp.
- A build from before the folders kept everything at `yinyue/` itself. On the
  first run, whoever is signed in takes it: there is no telling whose it was.
- Settings → Memory says whose lines it shows.

## Settings → Memory

A row on Settings opens the page, the same pattern as Notifications. Rows as
items, never raw markdown:

- **About them** — tap to edit, item menu to delete.
- Blocks per app — edit and delete alike.
- The Mac's copies sit in the same blocks she reads them in, marked "From
  your Mac", and take delete only. Deleting one deletes the Mac's row through
  the same door; the row is the user's.
- **From your Mac** — one line that says the truth (no Mac paired, not in
  reach, when the copy last came down) and a *Sync now* button (2026-09-09,
  Liang: the user must be able to sync from here). The run answers in words
  either way — what came down, what went up, or "Your Mac isn't connected" —
  and the rows are re-read so the screen matches the file. Never gated.
- Footer: when the last pass ran, how many rows she keeps, the cap, the model.

A delete stays deleted: it removes the row, and the Mac row behind it when
there is one. With the Mac away, a tombstone keeps the pull from bringing the
row back until the next sync reaps it there. An edit is the user's voice: the
dream keeps an edited row verbatim and never merges over it. Yinyue reads the
same rows, so a change is in her next turn with nothing to sync.

This screen shows her memory, not a memory browser. No search, no calendar,
no tiers to pick between; those stay on the Mac console.

## Build order

1. Store: account columns; the engine door stamps the actor; re-stamp on
   sign-in; a pull that answers core plus app-tagged long-term for one account.
2. Phone store: rows with an app tag and a Mac id, cap and eviction, the note
   tool's `app` argument, prompt rendering by block.
3. The nightly pass on the phone, riding the background wake, then push and
   pull.
4. Settings → Memory, and the model picker it shares with Yinyue.

## Open

- The Mac's dream still judges the owner's rows only. A second person's rows
  land stamped and wait; the dream needs to walk the store's `accounts` and
  run once per person.
- Whether a phone-only user's About them should seed the Mac's core tier at
  first pairing, or wait for the Mac's dream to judge it like any other row.
  Today it waits: the rows go up as short-term like any note.
