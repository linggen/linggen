# Asset packs — a skill's heavy files, fetched after install

**Status: engine side built 2026-11-15** (`src/assets/`; the phone, the
publish script and the packaging step are not). Part of 2.3 (`plan-3.0.md`,
Slimming). The first user is 灵境's art; the engine names no app.

What the build settled or changed from the design:

- **Object name.** `<hash>.<ext>` where `<ext>` is the path's extension *as
  written* when it is 1 to 8 ASCII letters or digits, else none (`<hash>`
  alone). The publisher must name objects by the same rule: it is part of the
  URL. The store keeps the extension so a stored object still has its content
  type.
- **Store path.** `<linggen home>/assets/sha256/<hash>.<ext>` (the design
  said `<hash>`).
- **Manifest checks.** `version` is 1, `base` is `https://`, every `path` is
  relative with no empty, `.` or `..` part, `sha256` is 64 hex digits, a path
  is listed once. `pack` and `order` are required on every file. A manifest
  that fails is logged once and the skill carries on as if it had none.
- **Fetching.** Started when the skills are loaded or reloaded (not on a
  timer), at most 4 downloads at once, lowest `order` first. A download that
  fails because the service cannot be had (no network, a 5xx, a full disk)
  pauses all fetching, 2 s doubling to 10 min; a 4xx or wrong bytes back off
  that file alone. One log line per burst, and only when it changes (the first
  failure, a recovery) — never one per file.
- **A page asking** puts that file first and sends it through any back-off or
  pause, once per ask.
- **State.** `GET /api/assets/<skill>` answers 404 for a skill with no
  manifest. `held` counts a file the skill's own folder holds as well as a
  stored one; `failing` is a wanted file that has failed since its last try.
- **Sweep.** Once, after the first load of the skills, and only if every
  declared manifest was read. An object nothing lists is kept a month before
  it goes, so an engine that sees another set of skills (`--skills-dir`) does
  not take what the first one still wants.
- Not built: a hole between the engine starting and the first manifest read
  (milliseconds) in which a page's ask is not queued — the first read queues
  everything anyway.

## The problem

A skill's page is small; its pictures are not. 灵境 is 209 MB in the app, 146 MB
of it art, and the iPhone app carries 106 MB of the same. The page must keep
working offline and must not change how it asks for a picture.

## The rule

A skill ships a **manifest** of its heavy files. A file the manifest lists and
the install does not hold is fetched in the background, checked, and kept on
the device. The skill's page asks for it at the same address as before.

- **Nothing in the page changes.** The engine serves `/apps/<skill>/<path>`
  from the skill's folder; when the file is not there, from the fetched copy.
- **A missing picture is not an error.** The page shows what it shows today
  for a picture that failed to load; the fetch goes on and the next paint has
  it.
- **Git stays the source.** The art lives in the skills repo as today. A
  publish step copies it to storage; packaging leaves the fetched part out.

## The manifest

`assets.json` at the skill's root, written by a tool, never by hand:

```json
{
  "version": 1,
  "base": "https://assets.linggen.dev/a/",
  "files": [
    { "path": "worlds/jiuding/art/places/shanmen.webp",
      "sha256": "…", "size": 184320, "pack": "ch02", "order": 20 }
  ]
}
```

- `path` — where the page asks for it, relative to the skill's folder.
- `sha256`, `size` — what the bytes must be. A fetched file that does not
  match is thrown away and tried again; it is never served.
- `pack` — a name the skill chooses (a chapter, "shared"). `order` — lower is
  fetched first. The engine reads neither as anything but a label and a number.
- A file the package holds is simply present in the folder; the manifest may
  list it or not.

The skill's frontmatter names it: `app.assets: assets.json`.

## Storage — by content, never by name

An object's address is its hash: `<base><first two hex>/<sha256>.<ext>`.

- An object never changes, so it can be cached for ever and a copy is as good
  as the original.
- A new version of a picture is a new object; the old one stays for installs
  that still ask for it.
- Publishing is idempotent: upload what is not there.

## On the device

- Fetched files live under `~/.linggen/assets/sha256/<hash>.<ext>`, shared by every
  skill and every version of one. A skill update fetches only what changed.
- **When.** After the engine has started and the skill is installed: every
  listed file not yet held, in `order`, a few at a time, in the background.
  A file the page asks for now jumps the queue.
- **Failure.** No network, storage unreachable (China), a bad hash: wait and
  try again, with a growing pause. The page is never told to wait.
- **What is known.** `GET /api/assets/<skill>` → files and bytes wanted, held
  and failing, per pack. A page may read it; nothing requires it to.
- **Removal.** Once no installed skill has listed an object for a month, a
  sweep removes it. Not before.

## The phone

The same manifest and the same storage; the phone's pictures are phone-sized,
so they are objects of their own. Held in the app's support folder.

- Wi-Fi: everything, in `order`, after first launch.
- Cellular: only the pack that comes next (the skill says which, as it knows
  where the player stands); the rest waits for Wi-Fi.

## Publishing and packaging

- `skills/scripts/publish-assets` builds the manifest from the skill's
  folder and a list of what stays in the package, and uploads the objects
  storage does not have. It prints what it would upload first; uploading needs
  Hanli's yes each time, as any publish does.
- App packaging (Mac and iPhone) copies the skill without the files the
  manifest marks as fetched. A marketplace install from the repo holds every
  file and fetches nothing.

## What stays in the package (灵境)

The first chapter's art and the art every chapter shares, ≈ 45 MB (estimate,
a lower bound). The list is proved, not guessed: the play bot plays 序章 and
第一章 on an install holding only that list, with fetching blocked, and
raises a finding for any picture that fails to load.

## Not in this design

- Model weights. They have their own sources and pins (`plan-3.0.md` §4).
- Code. A skill's scripts are never fetched this way.
