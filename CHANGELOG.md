# Changelog

## [2.5.2] - unreleased — the Mac app shows the web UI; rule files can import

### One UI

- **The Mac app's window shows the web UI itself.** Sessions, chat and every
  settings page are the pages a browser shows; the app's separate face is
  gone, and with it the second copy of settings.
- **Apps are tabs in the Mac app.** Chat first, then each app. An opened app
  stays loaded, so going from DJ to CFO and back finds both as they were. In a
  browser nothing changes: an app opens in a new browser tab.
- **Settings is one full page**, its sections declared once. Memory, Tools and
  MCP, Devices, Phone and Data are now reachable from the Mac app; the Desktop
  section shows only there.
- **An app declares its own settings page** (`app.settings` in its `SKILL.md`)
  and appears under Settings → Apps. No list in the engine names an app.
- **Escape leaves settings**, never while you are typing in a field or an
  editor.
- **A browser opens a launched app only for the session it shows.** In the Mac
  app a launch brings that app's tab forward and raises the window.

### Rule files

- **`@path` in a rule file imports that file** (`AGENTS.md`, `CLAUDE.md`,
  `CLAUDE.local.md`, `.cursorrules`): relative to the file that names it, up
  to five levels deep, never from a code span or block, and only when the path
  names a file — an `@name` mention stays as written.
- **`CLAUDE.local.md` is read** beside `CLAUDE.md`.
- **Your own rule file, `~/.claude/CLAUDE.md`, is read in a coding session**,
  before a project's rules, with its imports. Yinyue's turns and an app's
  turns never get it, even inside a coding session; nor does a plain chat
  with Ling. Its imports may name a file outside the project, wherever the
  session may read without asking; a protected place stays out.
- **An import is a read nobody was asked about**, so it brings in only what the
  session may read without asking, and never a protected place. A project's
  rule file imports only from inside the project; a rule file that is a link
  follows the same rule — so a project's `CLAUDE.md` that links to a file
  outside the project is no longer read, and says so in its place.
- **A rule that is not loaded says so** in its place — refused, too large
  (256 KiB a file, 512 KiB in all), not text.
- **A change to a parent folder's rules or to an imported file** is picked up
  on the next turn.

### Permissions

- **The folder a caller hands a run follows one rule**, for `write_dir` as for
  a preset grant: an existing absolute folder, never `/`, the home folder, or
  Linggen's own folder (or one inside it or holding it).

### Claude sessions are Linggen sessions

- **The Claude Agent SDK session is gone.** A Claude model runs in a Linggen
  session, with an API key or your Claude sign-in. If the Claude sessions'
  default was the only default you had set, it becomes your default.
- **An old Claude session opens read-only.** It keeps its history, renames,
  clears and deletes. **Continue in Linggen** starts a new Coding session in
  its folder with its conversation (your messages and Claude's replies; a
  long reply keeps its opening and its closing; tool calls, pictures and
  subagents' output stay behind, and the first row says so). It runs on the
  model the old session used when you still have it, else the first Claude
  model that can run now, else your default; the first row and a notice name
  it. Clearing an old Claude session also removes its resume pointer.
- **A message you had queued in an old Claude session before the upgrade is
  not sent.** It is kept in `~/.linggen/restart/unrecovered-<time>.json` and
  Yinyue tells you.
- **No longer supported:**
  - Claude Code hooks.
  - A project's `.mcp.json`: add its servers in Settings → Tools and MCP.
  - A worktree made automatically for each session.
- The SDK a Claude session downloaded stays in
  `~/.linggen/runtimes/claude-agent-sdk` (about 280 MB). Nothing uses it now,
  but it holds the conversations of your old Claude sessions, cleared and
  deleted ones too; delete it when you want them gone.

## [2.5.1] - 2026-11-20 — a coding session that asks less and says what it cost

### Permissions

- **Looking is free in a coding turn.** A command proven to only read
  (`ls`, `cat`, `grep`, `git log`, a glob, a recursive search) runs without
  a question, in the session's own folder and outside it.
- **Protected places always ask.** `~/.ssh`, `~/.gnupg`, cloud credentials,
  `.env` files and keychains are never covered by a wildcard or a home-wide
  grant; only a grant that names the place, or the person's own answer.
- **Linggen's own secret files are read only by a grant naming the file**, or
  by the person at this Mac saying yes. A delegated or unattended run is
  refused.
- **The gate follows `cd`.** `cd ui && …; cd .. && git status` is judged where
  each part really runs.

### Yinyue

- **She speaks only what she marks as speech.** A turn the engine started for
  her shows a line only when her reply ends with one; her reasons for staying
  quiet are never shown or saved.
- **A reminder is spent only where it was seen.** A condition counts as told
  when its line was drawn on the page in front, not when it was written.
- **Your message comes first.** Typing to her ends a turn the engine started
  for her; it never ends work you asked Ling to do.

### Working with models

- **Claude models get their real output limit**, asked from the provider per
  model, in place of a fixed 8,192; `max_output_tokens` in a model's config
  overrides it.
- **A tool call cut at the limit is told to make a smaller one.**
- **A coding turn may take 1,000 steps** (`agent.coding_max_iters`); a run that
  reaches it says so.
- **A session on a pay-per-token model shows what it used and an estimate of
  what it cost**, in the top bar and in `agent_status op=session`. Every call
  that reported usage is counted, stopped and failed ones too; a call's cost
  is fixed when it is made. A `price` in a model's config always wins; a
  model with no known price shows tokens and no dollars.

### Configuration

- **A config file that cannot be read is never written over.** It is set
  aside under a new name before a save, and `ling status` says so.
- **A wrong `price` costs only itself.** At load that model simply has no
  price, and `ling status` names it; a save refuses it.

## [2.5.0] - 2026-11-19 — one MCP door

One door for Claude Code and Codex, and the fixes of a whole-code review.

- **One MCP door at /mcp with twelve tools** (memory 5, agents 4, browser 3),
  in place of two servers with 41 tools. A tool with several jobs takes an
  `op`.
- **The X tools left the door.** The browser bridge's X module stays for
  Linggen's own apps.
- **`agent_status op=models` says which model can be used now** and, for one
  that cannot, why and what to do.
- **A run that stops on a usage limit says so**, with its reset time
  (`cannot_run`); the server text tells a caller not to retry before it.
- **An old plugin is told to update.** `/mcp/memory` answers 410
  `UPGRADE_PLUGIN`; nothing is forwarded. The engine remembers which host
  called, and `ling status` and Yinyue tell the person how to update it.
- **An approved computer sees the five memory tools only.**
- **Built-in missions refresh on startup** without resetting schedules,
  on/off settings, or model choices; their instructions are the engine's,
  so an edit to them is refused with how to copy one instead.
- **Codex sheds the old memory server.** When Codex starts the plugin, the
  retired `memory` entry an older plugin wrote is removed, and nothing else.

### Memory

- **An edited row is found by its new meaning at once.**
- **A row moved between long-term and short-term is never lost on the way.**
- **A dream marks a day done only for the rows it read.**
- **Memory mends a cut-off last change** when it opens after a crash.
- **Only the dated store snapshots count toward the seven kept backups.**
- **Another machine's queued memory writes are kept** until they land, and a
  queue that fails says so.
- **`ling mem --older-than` refuses a duration it cannot read.**

### Working with models

- **A reply cut at the output limit is seen as cut.** Three in a row end the
  turn as an output-limit failure.
- **A run that ends on an error keeps its work** for the turn that continues
  it, and says whether it was kept and why it could not go on.
- **A long, healthy stream is no longer cut off**; only a stream gone quiet
  times out.
- **A model with no key is shown as not ready.**
- **"Context too long" is recognised** in each provider's own words, and an
  Ollama error mid-stream shows as Ollama's.
- **The ChatGPT sign-in refreshes once at a time**, and a failed model-list
  fetch backs off.
- **A model server on your LAN counts as local.**
- **Clear on the default model keeps its fallbacks.**
- **A summary is applied only to the history it was made from.**

### Tools

- **Bash returns when its shell ends**, and what a call keeps is capped.
- **Glob lists the newest matches first.**
- **Read counts a file as read only when the read worked.**
- **WebFetch reaches public addresses only, and so do answer pictures**; it
  follows your proxy settings, gives a failed status with its code and reads
  a page up to a fixed size.
- **A skill tool's timeout bounds the whole call**, and its output is capped
  as it is read.
- **A skill installs whole or not at all**, and a failed install script fails
  the install.
- **A plan is approved once**, fails when its run fails, and goes back to
  planned when you stop the run.

### The web UI

- **A draft belongs to its chat.** Words typed while New chat is being made
  go to the new chat, and a late answer never lands in another one.
- **A stopped run shows as interrupted**, with no error line.
- **Nothing inside a code fence is rewritten.**
- **Enter presses the focused button** in a confirm dialog.
- **A refused save says so.** A key or room setting that was not saved shows
  as failed, and a batch delete removes only what was deleted.
- **One editor per mission**; deleting the mission closes it.
- **The day turns at 04:00 local time**, on a clock-change day too.

### Who may reach what

- **Only the companion can pass on your answer to a permission prompt**, and
  only for the listed tools.
- **A page from another site needs an allowed origin** to call the engine,
  except a page load and the sign-in's return.
- **Pairing codes allow few tries.** Wrong codes are counted per device and
  overall, with a growing wait.
- **A run is stopped only by the person whose session it is**, and a
  person's list holds only the sessions they may enter.
- **A saved key goes only to the address it was saved for.**
- **A path stays inside its folder** after it is normalised and its links
  followed.
- **Permission changes are saved whole before they apply**, with real paths;
  declining a home-folder request covers that path only.
- **Yinyue is handed only what your own sessions hold**, and a device's
  retained readings follow the same rule as when they were sent.
- **A sign-in is completed only by the request that started it.**
- **`agent_session_close` removes only the worktree its run was started
  with.**

### Under the hood

- An unreadable notes, push or mission-choices file is set aside, never
  written over; a mission whose choices cannot be read stays off.
- Files are written whole and swapped in, and every file with several
  writers has one lock (missions' history, devices, media, memory sidecars,
  perception).
- A mission's catch-up cap counts catch-up runs only.
- A day's telemetry digest is deleted once sent; log clean-up removes only
  the rolling log's own files.
- `ling stop` and `ling status` act on the recorded pid only when it is the
  engine.
- A bash app started by `RunApp` runs like any other command the engine
  starts; a line's voice must be one of the listed voices.

## [2.4.0] - 2026-11-18 — one Ling and one Yinyue

On the Mac now; the iPhone's 2.4 follows.

- **Memory warms up in the background.** A write or search in the first
  seconds waits for it (up to 20 s); automatic recall skips the turn instead.
- **One Ling and one Yinyue across apps.** App chats, Yinyue's moments and
  the phone's hand-offs share one conversation a day — Linggen's daily
  session. It is on; Settings → Session turns it off.
- **A private app keeps its talk to itself.** An app that declares its talk
  private, such as Money, keeps its own chat and never joins the day.
- **Each person has a day of their own.** A phone signed in to another account
  gets that person's day, never yours.
- **In the day, each app's page is still that app.** A `!` command starts in
  the app's folder, Copy system prompt gives the app's prompt, and Yinyue
  keeps her own tools.
- **A page says whose work it shows.** Work from another app reads "Working in
  Music", folds to one line, and its waiting message can be changed only there.
- **One model a day.** The model picker sets it; a page opened earlier no
  longer changes it back.
- **Clear takes only this page's part of the day.** It stops no running turn
  and never removes the day's summary.
- **Every chat sees the same world.** Ling's chats, app chats and Yinyue's
  thread are shown the page you are on, the machine and the phone, and each
  app adds a line of its own.
- **Yinyue and Ling keep notes.** Notes ride beside their next turn, and she
  can ask to look again at a set time.
- **Yinyue speaks aloud only where you cannot see her.** On the page where
  her line shows it is text only; anywhere else she is heard.
- **Rooms are off.** Nothing about rooms shows and every way in says so;
  `rooms_on = true` in `room_config.toml` brings them back.
- **What Yinyue says unprompted has limits.** At most 6 lines an hour and none
  from 23:00 to 07:00 by default, both set in Settings → Agents.
- **A failed mission waits for the morning.** Yinyue is told once, and the
  fault is never dropped.
- **The activity log keeps 400 days.** Ask it by day, app, verb or text, and
  for totals of time spent in each app.
- **Coding sessions.** Switch Coding on in a chat, or start an `agent_run`
  with a folder to write, and the session works on code with care.
- **A coding session runs its work in its folder without asking.** Anything
  outside the folder, or leaving the machine, still asks first.
- **The gate says why.** When a command asks or is blocked, the agent gets
  the reason.
- **Inline code follows the same rule as a script.** In a coding session,
  the gate judges interpreter code the same way whether it is written on the
  command line or in a script in the folder.
- **A read grant lets the agent inspect lines.** Plain `sed -n` line selections
  work without asking, including after changing into a granted folder.
- **Hand-offs keep their brief.** Linggen recalls in the delegated session;
  the plugin no longer rewrites the call.
- **A caller's words are not yours.** A delegated brief cannot give a memory
  the standing of something you said.
- **Outward commands always ask, once.** Publishing, ssh and a push to main
  ask whatever you have granted.
- **Fewer questions for harmless commands.** Running a script in the folder,
  or copying a file out of somewhere you may read, no longer asks; `$HOME` is
  read as the shell reads it.
- **More of an ordinary command is read.** A `git grep` pattern, `$?`, a
  loop over words written in the command, and `if … then … fi` are judged
  by what they run.
- **Permission checks read a command more exactly.** A program or a path is
  recognised however it is quoted or cased, on every line of a command.
- **A plugin run can come with its folders preset.** `agent_run` takes the
  folders and tools a new run may use without asking, within the caller's own
  level.
- **A paired phone's standing follows its account.** A phone signed in to
  your Mac's account is yours, and any other phone starts at read.
- **You set another person's phone on the Mac.** Settings → Phone shows whose
  each device is and lets you choose Chat, Read or Edit there, and only there.
- **A phone that is not yours gets none of your memory, models or keys,** and
  permission checks were tightened on every route a phone can use.
- **A phone and a Mac too far apart in version say so once**, in plain words.
- **The phone's Settings shows what it may do on its Mac.** A refusal is
  worded by Yinyue.
- **Yinyue says a problem once, with something to do about it.** She raises it
  again only when it worsens or comes back, with buttons that fix it.
- **A tap runs the fix as your own button.** A destructive one asks first,
  and "not now" keeps a problem quiet for a week unless it gets worse.
- **She never drops a problem.** One she sat out, or could not be asked about,
  is asked again later.
- **The phone offers to free its storage.** Take the songs off the phone, clear
  leftover share pictures, or delete photos already backed up to the Mac.
- **Apple Shifu offers a disk scan** when the Mac's disk runs out.
- **A Mac out of reach is news, not a problem.** A sleeping Mac no longer
  makes her raise it.
- **GPT models follow new generations by themselves.** A ChatGPT model family
  moves to its newer model once that has answered, and Sol is now GPT-6.2.
- **Models tab: stay or follow.** It says when a ChatGPT family moved and lets
  you stay on the previous model.
- **A paired phone takes the ChatGPT version and old model names from the
  Mac**, and a model you added there is asked for by its provider's name.
- **The phone sends the Mac no shell text.** Music, Money, Health and
  灵境's book sync through tools the apps declare.
- **The iPhone app is smaller.** 灵境's later chapters are fetched after
  install, taking its part of the app from 106.7 to 86.1 MB (measured).
- **Chat on the phone** shows Ling's and Yinyue's notes while the day is on,
  and a hand-off waits only for its own turn.
- **Write no longer overwrites a file** that changed since the session last
  saw it; it asks for a fresh read first.
- **Stopping a background task** right after its turn ended no longer fails.
- **A long file with non-Latin text** no longer stops the turn that writes it.
- **The models list shows a model's settings, never its key.**
- **A request from a paired device on the LAN must carry a JSON object.**
- **An agent set to a retired model** now runs on its successor.
- **A long name on a crowded 灵境 field card** no longer runs into its status.
- **灵境: a won story fight shows one 所得 card** with everything it paid, and
  啸月 is greyed while none of his bodies stands.

### For developers / under the hood

- Every kind of session is one spec applied in one place (`session-spec.md`),
  and a turn's place, settings and grants are set only in the listed places,
  each held by a test.
- One rule decides which model a turn runs on (`model_choice::resolve`, in
  `doc/models.md`); the ChatGPT catalog lives in one module.
- One `Person` type says who a request, a session or a row is for, and one
  ladder of permission levels; the legacy tool permission setting is retired.
- A route has a class, and a request above its caller's level is refused;
  each phone and Mac carry a contract number kept equal by a test.
- Skills may declare `world-line`, `activity-verbs` and `private-talk`, and
  phone sync as `page_only` tools.
- An app's page removes only its own app's sessions: `DELETE
  /api/skill-sessions` must name the session's skill.
- In a day's session, run and stream events, the running run and queued
  messages carry `place`; a page's `compact_config` is not stored there.
- `evals/code` runs sixteen small tasks with hidden checks three ways: with the
  coding section, without it, and in a Claude Code session.
- The small helpers have one copy each, and a session file's changes go
  through one writer; the agent wait socket's `finished` frame carries
  `followup`.

## [2.3.1] - 2026-11-16 — memory on newer embedding and vector-database libraries

The two major library groups held back from 2.3.0; nothing else changed.

- **Embedding stack:** fastembed 5.13.4 → 5.17.4 on candle 0.10.2 → 0.11.0
  (ort 2.0.0-rc.12 → rc.13). The embedding vectors are bit-equal to 2.3.0's,
  so search over stored rows is unchanged. The Kokoro voice stays on candle
  0.10 until its library moves.
- **Vector database:** lancedb 0.31 → 0.41, lance 8.0.0 → 14.0.2,
  datafusion 53.1 → 54.1; arrow stays 58.3. New tables are still written in
  Lance file format 2.1 (lancedb 0.41 would default to 2.2).
- **Your store:** 2.3.1 opens a 2.3.0 store as it is, rewrites nothing, and
  takes its one backup before first opening it
  (`memory/backups/takeover-<stamp>/`). Going back to 2.3.0 afterwards
  works: it opens the store with every row, including rows 2.3.1 wrote.

## [2.3.0] - 2026-11-16 — memory is part of ling, one program

- **Settings → Memory.** The memory console is a tab of the engine's own
  settings: Rows (search by meaning, filter, add, edit, delete), Days (a
  month calendar; "Scan this day", "Dream this day", "Dream now"), Review
  (dismiss an item, or start a solve) and Settings (how long short-term rows
  are kept, the recall threshold, other machines). It fits a phone-width
  window and follows the page language. The separate `shared-memory` app is
  no longer bundled.
- **The engine calls the memory it hosts directly.** Memory is a library with
  one entry point and one table of verbs; the engine's own calls no longer
  travel over loopback HTTP, a caller is named on every call, and a phone
  that is another person is fitted to its own rows inside memory. Memory's
  HTTP and MCP routes stay for plugins, the CLI and other machines.
- **Claude Code and Codex are the plugin's hosts.** OpenClaw is no longer
  supported. The plugin bundle (`linggen-plugin.tar.gz`) is an asset of the
  engine's release.
- **One license.** Everything in this repository — the memory crate, the
  plugin with its skill, the browser extension — is Apache 2.0.
- **Scan and solve are started from the engine.** Two built-in missions on
  the memory agent, with no schedule: `memory-scan` (trigger with
  `{"day":"YYYY-MM-DD","attended":true}`) backfills one day from this
  machine's session logs into short-term memory, skipping sessions that
  already have rows, and stamps the day scanned; `memory-solve` (trigger with
  `{"attended":true}`) works the review queue from evidence and asks the user
  only what evidence cannot settle. A second start while one runs gets 409.
  The engine itself runs the walk of the session logs before the scan's
  first turn; the scan agent has no shell and reads only the candidates
  file. The solve reads only, like the dream.
- **`ling mem scan <day>`.** The session scan (Claude Code, Codex, OpenClaw,
  Linggen logs → `~/.linggen/memory/.scan-output.jsonl`) is part of `ling`:
  the same file, header line and exit codes as the scripts it replaces, with
  the engine up or stopped. It needs `bash`, `jq` and `perl`.
- **Built-in missions are seeded once each.** An upgrade that brings a new
  built-in mission adds it; a built-in the user deleted stays deleted.
- **Missions can declare a `prepare` step**: arguments to the engine's own
  binary (`[mem, scan, $DAY]`), run by the engine when a run starts, before
  any model call; a failure ends the run as failed, its exit status and
  error tail in the run record.
- **The plugin and the browser extension live in this repo.** The `linggen`
  plugin for Claude Code and Codex is `plugins/linggen` (install
  `linggen@linggen` from this repo's marketplace files) and the Chrome
  extension is `browser-extension/`; the plugin has the engine's version, and
  its session scan is `ling mem scan <day>`.
- **A mission run that had something fail no longer reads as a plain
  success.** Its `runs.jsonl` entry keeps each failed tool call (a declared
  script's non-zero exit, a tool error, a refused call: tool, status, the last
  line or two of its error output, capped), a step-limit stop and a failed
  run's error; the missions page and the toast mark it, the activity log gets
  one line, and an unattended run hands Yinyue the facts — including how many
  runs in a row showed the same failure — for her to say or not, held until
  someone is at a screen. A clean
  mission run no longer wakes her.
- **The engine stays up.** It hosts memory now, and other tools on the machine
  use memory whether or not a Linggen window is open, so it no longer exits
  when idle. `--idle-shutdown-secs` is accepted and ignored.
- **Asset packs, engine side.** A skill can list heavy files in a manifest
  (`app.assets`); the ones its folder lacks are fetched in the background by
  hash, checked, kept in `~/.linggen/assets/sha256/`, and served at the same
  `/apps/<skill>/…` address. `GET /api/assets/<skill>` reads what is held.
- **A rollback brings the old `ling-mem` back.** `ling update --rollback` (and
  the roll-back of a restart whose engine does not come up) to an engine that
  does not host memory renames `ling-mem.prev` over the stand-in before that
  engine starts, so it can start memory as before; rolling forward takes the
  store over again.
- **Memory runs inside the engine** (`doc/plan-3.0.md`, step 2.1). The memory code
  (`crates/linggen-memory`, from ling-mem 1.13.0) is part of the build; `ling`
  opens the store at start-up and serves memory on its own port —
  `/mem/api/memory/<verb>` and `/mcp/memory` for this machine, the phone's
  `/api/memory/<verb>` as before. No separate `ling-mem` is started,
  installed or looked for. `ling mem <verb>` runs memory's data verbs,
  with the engine up or down (stats, days and the other handler verbs too).
- **One restart cuts a machine over.** At start-up the engine stops a
  `ling-mem` daemon still serving the store, copies the store once per
  version (`memory/backups/takeover-…`), and holds it under
  `memory/.store.lock`. `~/.local/bin/ling-mem` becomes a stand-in for
  `ling mem` (the real binary stays as `ling-mem.prev` for a rollback).
- **One port.** The engine listens on its own port only (9527, or
  `--port` / `LINGGEN_PORT`); nothing answers on 9528. The plugins move to
  the engine's port. `[agent].ling_mem_url` and `$LING_MEM_URL` are gone:
  the engine's callers use the store it hosts, and an old config's key is
  ignored.
- **Memory across machines: approved machines only.** Each paired device is
  a `phone` or a `machine` (approved with Allow; existing rows are
  classified by how they were made). A machine is the owner's other
  computer and sees all of the owner's memory: over the LAN, with its token,
  it reaches `/mcp/memory` (for a machine with only a plugin) and the
  replica protocol under `/mem`. A phone sees only its person's rows, at
  `/api/memory/<verb>` and from skill pages, and is refused on the machine
  routes. One rule decides this for every memory route. Replicas now call
  the primary at its Linggen port (`host:9527`); a replica set up against
  the old daemon's `host:9528` pairs again.
- **An engine uses its own memory.** "Use another machine's store by
  address" is gone (Settings → Memory → "Ling-mem URL" and its routes); an
  old config naming one is ignored. Share memory with Primary instead. A plugin-only machine
  still points its plugin at another machine.
- **Allow / Deny at this Mac, in its web UI.** The local page's channel
  says it was opened on this machine, so the Share-memory prompt shows
  there; a phone's or a relay peer's channel never counts as this machine.
- **A phone that is another person runs only per-person memory verbs.**
  `/api/memory/<verb>` and skill pages allow it `search`, `list`, `count`,
  `get`, `add`, `add_batch`, `update`, `delete`, `forget` and
  `session_start`; memory keeps its `replace_ids`, duplicate check and
  batch rows inside that person's rows.
- **Phones and the owner's doors.** The MCP front door (`/mcp`) is for this
  machine and approved machines; a phone is refused. `/api/user/name` gives
  each caller its own name. A phone's first requests wait up to 1.5 s for it
  to name its device.

## [2.2.1] - 2026-11-14 — the permission gate reads a command as the shell does

- **The permission gate reads a command the way the shell does.** Text a
  command carries is no longer taken for a path: a heredoc body written with
  `cat` or `tee`, and the pattern of `grep`, `rg`, `sed`, `awk` or `jq` — so
  writing notes or searching for `/api/…` stops asking. It is dropped only
  where the gate and the shell agree on every line; anything unclear asks.
- **Holes in the gate closed.** A redirection glued to its file (`>/path`,
  `</path`) is a target; a command that runs code is judged where it runs even
  when its only paths are in the temp folder (`sh /tmp/x.sh` no longer runs in
  a read-only session); a `sed` or `awk` script that names a file or is read
  differently by BSD and GNU tools asks; a quote inside a comment no longer
  hides the command after it.
- **A session reads its own folder, and `sleep` only waits.** An agent can
  watch its own background task's log without a prompt.
- **`agent_permit` shows the whole command.** A permission request from an
  `agent_run` session carries the full Bash text when its summary is cut, and
  a long command with CJK text no longer crashes the prompt.
- **`agent_status` answers the caller's prompt.** `reply` is the agent's last
  message in the turn that answered you; a later turn the engine started
  itself (a background task ending) is `followup`. A watch that expires while
  the run goes on is re-armed.
- **The previous binary stays out of the app bundle.** `ling update` and
  `ling restart` keep `ling.prev` under `~/.linggen/prev/` for a binary inside
  a macOS `.app`, so the bundle's seal holds; one left inside is moved out.
- **Linux.** A leftover background shell with a multi-line command is found
  and ended on Linux too (`ps` prints a newline differently there); a session
  list with ties comes back in a fixed order.

## [2.2.0] - 2026-11-13 — 灵境: cards worth collecting, and 灵石 with a use

- **Cards have a 品 and a frame.** Every card wears the frame of its grade
  (凡, 银, 金, 宝石, 钻石), read off where the card is found, and the 卡册 beside
  the 牌组 shows what you hold and what is still out there.
- **New card mechanics in 斗法.** Shield, poison, laid talismans (伏符) and
  root skills (灵根术) play in fights, and each card's face says what it does.
- **银月 coaches a lost duel.** After a loss she names the move that would
  have won and the lesson of the fight; after losses in a row she lends her own
  cards into your opening hand. A 秘境 boss can be fought again once.
- **灵石 has a use.** 闭关 can burn 灵石 in a 聚灵阵 to gather faster, with a
  preview of what each choice pays; a 秘境 run costs 体力 at the door instead of
  a 秘境令; the 饭桶 eats 炉炭 from the pill shelves. 修为 stops at the cap and
  the rest is paid as 灵石. Saves move to version 19.
- **The day's work is easier to read.** The tray shows three rows and one
  goal, with 全部 for the rest; daily chores rotate; the gain float names the
  board that paid it; story prose gives pinyin for creatures out of a classic.
- **Hand seals hold through a flicker.** A seal survives a brief loss of
  recognition, and on a narrow screen the seal helper floats over the board.
- **银月 only speaks in a quiet moment.** A quiet glance may talk and look; it
  never starts work, never hands off into a session an agent or a person is
  using, and never claims you asked.
- **`agent_run` waits properly.** The wait stream stays open through short
  idle gaps, a finished run always wakes its caller, a permission request is
  delivered again to a new watcher, and a run that is just starting no longer
  looks finished.
- **Cloud save knows the newest rules it may hold.** A device declares the
  newest save version it plays, and a copy waits by progress, not by file time.
- **Smaller fixes.** Mission and skill sessions stay under Active for a day
  after they finish; the model bar's auto-switch is a pill like the others;
  the Mac's disk is named in what 银月 knows of right now.

## [2.1.0] - 2026-11-11 — 灵境 on its numbers, and the app keeps your skills current

- **灵境 plays on one set of numbers.** 修为, the layers, 灵石, items and fights
  now sit on one curve across 81 levels, so a night's 闭关, a small game and a
  fight each pay what the layer you stand at is worth. Saves move to version 18.
- **New play in 灵境.** 守山门's 兽潮 nights keep up the pressure with new side
  cards and beasts; every 秘境 room draws a different game; small games arrive
  as 观里的事, a place, a person and a reason; 褚先生 passes on a seal a week
  (师父传印); the sky shows the sun by day and the moon in its true phase, with
  the 节气 beside the weather in the header.
- **Save sync trusts the newer rules.** When the Mac and the cloud disagree, the
  save carried forward by newer rules wins, and the clock's own stamps no longer
  count as progress, so false conflict copies stop.
- **A save from a newer Linggen is left alone.** An older build that meets a
  save from newer rules never writes or migrates it; the page says
  「存档来自新版 Linggen，更新后继续。」
- **Linggen.app delivers skill updates.** An app update now gives installed
  skills the bundle's newer code and never touches their saved data.
- **An old engine is replaced after an update.** If the engine left running is
  older than the one in the new app, the app restarts it on the new one.
- **A session can refuse to switch models.** Each session has a switch (the
  chat's model bar: 出错时自动换模型 / Switch model automatically on errors).
  Off, a failed model call ends the run on that model, naming it and the
  provider's reason, instead of answering on another. Sessions a host creates
  through `agent_run` start with it off (`model_fallback: true` opts back in),
  and `agent_status` and the wait stream's last frame now report `status`,
  `error` and the `model` that answered.
- **Agents ask instead of failing.** An `agent_run` session that needs a
  permission asks its caller, who answers with `agent_permit`; a declined one
  closes that project for the run, and `agent_stop` ends a run that is waiting.
  A subagent's model fallback stays its own and no longer changes the main
  chat's model.
- **Smaller fixes.** A plan's checklist shows once while it runs; a failed
  Responses API stream keeps its real reason; `ling restart` works after an app
  reinstall replaced the binary; launching an app brings its tab and window up;
  the memory dream settles review items itself and asks you only when it must.
- Includes 2.0.2 (the engine fetches bun; the app restarts a silent engine).

## [2.0.2] - 2026-11-11 — 灵境 wakes on a Mac with no JS runtime

- **The engine fetches bun itself.** A Mac that got Linggen.app from the DMG
  runs no install script, so with no node it had no JS runtime: skill actions
  failed and 灵境 sat on 「灵境还没醒来」. The engine now downloads a pinned,
  checksum-verified bun into `~/.linggen/bin` when it is missing, and
  `/api/runtime/js` says where that stands.
- **Linggen.app restarts a silent engine.** The app now checks the engine every
  15 s; after about 45 s without an answer it starts the bundled engine again.

## [2.0.1] - 2026-11-08 — your iPhone finds your Mac again

- **「查找 Mac」 finds the Mac.** On macOS the engine now announces itself
  through the system's own Bonjour service (mDNSResponder). Run from
  Linggen.app, macOS had kept the engine's own announcer off the Wi-Fi, so
  phones nearby saw nothing and a saved pairing could not be reached by name.
- The watchdog that re-announced every few minutes is gone on macOS; it was
  answering a problem that never was. Linux keeps its own announcer.

## [2.0.0] - 2026-11-03 — Linggen 2.0：灵境

- **灵境 (Lingjing), the cultivation game, ships with Linggen.** Play it on the
  Mac beside the chat, with Ling telling the story and 银月 at your side, and
  on the iPhone with no Mac needed. One save, carried between them.
- Engine, Linggen.app and the iPhone app move to 2.0 together. The 1.10.0
  changes below (one memory across machines) were never published on their
  own and ship in this release.

Since the 11-03 draft:

- **Let Claude Code and Codex hand work to Ling.** Through the MCP front door,
  `agent_run` starts a Ling session, optionally in a folder Ling may write to.
  The run outlives the call and ends only when its own background work is done.
  `agent_status`, `agent_stop` and `agent_models` round it out, and
  `ws /api/agent/wait` pushes the end of a run. A delegated run keeps its
  checklist and can draw pictures. External sessions show in the session list.
- **One chat, one permission set.** A subagent uses the chat's grants live and
  saves new ones there. A folder inside the home directory can be granted
  wherever the home lives.
- **Session list: Active / History.** It replaces the creator tabs. A row spins
  while its subagents or background tasks run, and the header's running dot is
  gone.
- **Plans and todos work the way Claude Code's do.** Each step has an
  "in progress" wording, plans are short, and a session shows one checklist.
- **Restarts lose nothing.** A message queued, steered or half-delivered
  during an engine restart is handed to the new engine. Interrupted background
  work is listed in a continue turn. A tool block always ends with its turn,
  and a subagent's end always reaches the agent that started it.
- **Chat.** Steer works on a queued message, as in Claude Code. Tap an agent's
  name to reply to it. An app in its own language gets the chat's words and
  银月's lines in that language. Picture thumbnails show the whole picture.
  The weather chip turns the weather animation on and off. The chat bar shows
  the model's context limit and, on a ChatGPT model only, Codex's 5h/7d
  allowance.
- **Release gate plays 灵境.** The play bot runs on the skills being released,
  and the phone's 灵境 tests run on assets built from the same commit.

- **Found pictures match the question.** Ling sees an image search's
  pictures as one labelled contact sheet (on a model that sees) and picks by
  what they show. A page's cover picture is no longer captioned by the page
  title, and pages or pictures that share too few words with the search are
  left out.
- **Sources and pictures in answers** — in any session. Every web result and
  picture a tool returns (a search's sources, a fetched page's main picture
  and captioned pictures, a browser screenshot) gets a number such
  as `[turn2image1]`; an answer that writes one shows the source as a link
  chip or the picture as a captioned thumbnail that opens large. A number
  that matches nothing shows nothing. GPT models' own citation markers now
  resolve to these numbers instead of being dropped. Shown pictures are
  downloaded once, scaled to ~1000 px and kept with the session (30 MB a
  session; past it they load from their address). `check.sh live` guards it
  (case h).
- **Web search is text only again, and fast** (~2 s, not 10–12 s): pictures
  come from the pages Ling fetches. A search that cannot reach its service
  says WebFetch still works, instead of ending the answer. A page picture
  with no caption is named by its file's words (else the page title), and
  a number written as a markdown image or link is kept in the plain form.
- **Ling draws pictures in any chat.** Ask for a drawing and `GenerateImage`
  draws it with ChatGPT on your ChatGPT sign-in (about 30 s), else on this
  Mac's local picture model; the picture shows in the answer by its number
  and is deleted with the conversation. A ChatGPT usage limit is said in one
  line, never swapped for another source. Fixed: the tool was offered in
  plain chats where it could only fail; a tool that cannot run is no longer
  offered.
- Fixed: in a Claude session, a queued message handed to a runner that ended right after its last answer failed instead of running on a fresh one; a permission card that had just appeared could vanish while its run waited on it.

- **Read opens pictures.** A PNG, JPEG, GIF, WebP or HEIC file (known by
  its bytes, not its name) is no longer read as garbage text. A model that
  sees gets the picture once, scaled like a pasted one (1568 px, JPEG);
  a blind model gets the facts (format, size in pixels and bytes). Either
  way it is numbered and kept with the session, so the answer shows it even
  after the file changes or goes. Over 20 MB: facts only. Fixed: a JPEG,
  GIF or WebP sent to a model was always labelled PNG.
- **A run's content stays under its own message** — every event a run sends
  (tokens, text, tool blocks and output, its message, status, outcome, turn
  end) and every row it saves carries `run_id`; the chat binds a run's bubble
  to its saved rows by that id alone. A stop followed by a new request with
  the same tool, a mid-turn session switch, a reconnect or a reload no longer
  moves one turn's calls or answer under another. Sessions saved before keep
  the old matching. `LINGGEN_EVENT_LOG` tees UI events to a file for tests.

### From 1.10.0 (2026-10-21, never published) — one memory across machines

- **Memory across machines** (ling-mem 1.13.0). When a LAN machine asks to
  share this Mac's memory, Allow / Deny shows in the web UI; only someone
  at this Mac can answer (loopback, not through the phone's WebRTC tunnel,
  which now stamps `x-linggen-tunnel`). Settings → Memory → Primary sets this
  machine's primary; `~/.linggen/client.json` holds where ling-mem lives.
  The dream takes ling-mem's per-day lock.
- **One Linggen-wide language setting**; the engine's pages render in 中文.
- **Yinyue appears only once met** (a one-way latch); Ling continues her own
  tasks across devices, and Yinyue hands work to Ling.
- **Pictures in a turn** are kept with the row, sent to the model once, and
  shown as thumbnails that survive a reload.
- **Weather** — the sky over the whole UI: rain, fog, sun and moon, wind,
  the 24 节气 and festivals; the place found by IP.
- **Skills** — apps may declare `app.allow: [camera|microphone]`; sessions
  bound to declared params (`{{session.<name>}}`); missions take a `run-if`
  precondition; `remote: true` tools are published to the phone as a
  catalog; an app's title comes from its SKILL.md.
- **Ling answers in the model's own style** in a plain chat — no persona
  rule cuts a real question to a line; apps and skills still set their own.
  `check.sh live` guards it (case g).
- **No citation glyphs** — a ChatGPT model's private citation markers
  (`cite turn2search1` between boxes) are dropped from replies; url
  citations, when sent, follow as links.
- **Composer** — `@` addresses Ling or Yinyue, `/f` opens the file picker.
- **All WebRTC** — the web UI and skill pages have no HTTP data path; the UI
  reconnects when ICE stays down.
- **Rename in the session list saves cleanly** — an empty reply over the
  data channel is no body, so a save that answers a bare status resolves.
- **Settings** regrouped (Agents, Session + Memory); Anthropic is API-key
  only.
- **Linux** binaries build on Ubuntu 24.04 and need glibc 2.39+ (Ubuntu
  24.04 or newer).

## [1.9.0] - 2026-10-05 — shared sessions

- **Shared sessions** (`doc/shared-session-spec.md`). Several agents sit in
  one session: each member gets its own engine and its own thread built
  from the session file, one turn lock per session, one saved compaction,
  and the session's tool set and permissions apply to every member. `@name`
  routes a message to a member (longest name, CJK-aware). Guest mode is gone.
- **`ling update` keeps `ling.prev`** and puts it back when the new binary
  won't start; `ling update --rollback` swaps to it and back. After a swap
  the running engine restarts on the new binary. `LINGGEN_RELEASE_BASE=<url>`
  points every installer and updater at another release mirror
  (`doc/cli.md` § Release override) — for the release gate.
- **The mac `ling` ships Developer ID signed and notarized**, so a browser
  download opens; `release.sh` refuses an ad-hoc upload.
- **Senses a skill may declare** — `weather` (Open-Meteo at the city the
  person set, off until set) and `user_words` (the person's recent messages
  as typed); `reach` tells every skill which well-known services this
  machine cannot reach.
- **Reachable from China** — Qwen, Kimi, GLM and SiliconFlow presets;
  self-update, runtime, skills and Hugging Face fall back to mirrors; pip
  retries through mainland PyPI mirrors; STUN has a third server.
- **Skills ship missions, the engine runs them**; cron weekdays count from
  Monday (1-5 no longer ran Sunday to Thursday); a catch-up fills the missed
  slot.
- **Pictures** — `GenerateImage` on a local FLUX lane, installed on first
  use and only on a Mac with the memory and disk for it.
- **Chat** — suggestion buttons above the input, the next-prompt hint (Tab
  takes it), Resend on a failed send or an interrupted run, long chats
  render their last 100 messages, a run cut off before its reply shows its
  tool calls.
- **Hardening** — loopback callers must name a loopback Host and a trusted
  Origin; skill pages are framed and messaged only by their own origin; the
  media channel checks the peer's rights; file endpoints serve only known
  roots.
- **Removed** — `ling eval`, `capture_screenshot`/headless Chrome, routing
  policies that never ran, uncalled HTTP routes.
- **Memory scope and index** (ling-mem `doc/scope-index-spec.md`). Memory calls
  carry where the session stands — `cwd`, `root`, and on a search
  `cwd_scope` = root; a skill that uses memory stands in its own dir
  (`~/.linggen/skills/<name>`), replacing the forced `contexts` tag
  (`memory-context:` is now just the opt-in). The always-on block adds the
  "Memory scopes here" line and the index of standing rows for the session's
  directory, reloaded on cd; app sessions get their dir's index. A home chat's
  recall sees only rows about the person. Recall lines carry `from=`. The dream
  promotes with `tier=semantic` explicitly, carries hooks, and proposes index
  and scope changes as review items.
- **A skill declares its billing product** — `product: cfo` in SKILL.md
  names the Linggen Cloud product its turns bill to (`X-Linggen-App`). The
  engine's hard-coded list of app names is gone; a skill without one bills
  the shared bucket.
- **ChatGPT built-ins are GPT-6.1 Sol and GPT-6 Luna** — the three GPT-5.6
  models (Sol, Terra, Luna) and `gpt-6-sol` are retired; `gpt-6-luna` is the
  default. Saved defaults, Yinyue's model and each paired phone's model list
  move on load (5.6 Sol and 6 Sol → 6.1 Sol, Terra and Luna → 6 Luna), and
  any other saved id — a session, a mission's `model:`, a room share —
  resolves to its successor. The Codex backend refuses newer models under
  older clients, so requests now identify as Codex 0.159.2.
- **`ling update` verifies the download** — the release manifest carries
  each asset's sha256; a mismatch aborts before the binary is replaced.

## [1.8.2] - 2026-09-10 — a person's turn reaches the model verbatim

- **A chat turn is no longer wrapped as a task** — every turn used to arrive
  inside the autonomous-agent bootstrap ("Ignore any prior greetings…
  Workspace contents… explore the codebase before making changes"), a second
  copy of a message already in history, and a model that took it literally
  answered "hi" by crawling the workspace. `PromptProfile.task_bootstrap` is
  off for owner and consumer sessions and on only for missions, headless
  runs, evals and delegated subagents.
- **Keys belong to endpoints** — `credentials.json` was keyed by model id, so
  replacing a model row orphaned its key ("Missing Authorization" on a model
  configured in Settings). Entries are stamped with provider + url on save;
  resolution is own key → configured sibling on the same endpoint → any
  stamped row for it. Saving a key rebuilds the model manager, so it is live
  at once; `ling status` and the models tab read the same resolver.
- **Only Yinyue's final paragraph is spoken** — a reasoning model that
  deliberates above its answer no longer has the deliberation read aloud.
  Every engine-authored kickoff says so; one parser reads every reply: last
  paragraph, quotes unwrapped, SILENT (bare or as the last word) means
  silence. Discarded notes are logged.
- **MCP servers that weren't listening at boot are dialled again** — the
  engine pre-warms its own memory daemon seconds before connecting to it and
  could lose that race for the daemon's whole life (a day of empty
  auto-recall and a dream with no memory tools). A watcher re-dials only the
  enabled, unconnected servers, backing off 5s → 5min until everything is up.
- **The model picker steers a mission, and a refused model is benched** —
  on a mission session the picker wrote a per-session override no mission
  run read; it now sets the mission's own model. The fallback chain reads
  `resets_at` from a usage-limit error and benches that model until then
  (60s unnamed, an hour at most), so a turn never spends its last candidate
  on a known failure. A successful fallback pins the model on the session
  and the picker follows it.
- **A key the site refused reads as signed out** — the models page said
  signed in for two hours on a key linggen.dev had already rejected. The
  site's 401 verdict is remembered on the current key and cleared by a
  successful fetch or a newly saved key. This Mac signs in as one named
  device and signs only itself out; the phone signing in never signs the
  Mac out.
- **Whose memory a row is** — a paired phone's verbs are stamped with the
  person behind the pairing (read from the pairing record, never the body);
  the Mac's own account is the owner and unstamped; a signed-out phone
  writes as `device:<id>` and its rows are handed to the person at the next
  signed-in connect. Needs ling-mem 1.8.0. Settings → Memory gains Sync now.
- **A skill embed runs the session's model and never hides a finished
  reply** — "Ask Ling" on a Health card ran on the global default (and
  thrashed through fallbacks when it was rate-limited) because no list an
  embed could fetch carried the session's model; and a non-streamed reply
  stayed hidden behind an earlier preamble block under a stuck spinner.
- **Linggen Cloud is DeepSeek V4.1 Flash, under its own id** — the built-in
  model is now `deepseek-flash`, the id DeepSeek gave V4.1 Flash (1M context,
  ahead of V4 Pro, which DeepSeek routes to it from 2026-09-14). The retired
  `deepseek-v4-flash` is still honoured everywhere a model id is looked up —
  a saved session, a skill's `model:` pin, `pet.model` — and persisted
  defaults and pins are rewritten on load, so nothing falls silently to the
  default. Skills and the phone pin the new id; the proxy accepts both.
- **A model has vision when it says so, or when its family does** — DeepSeek
  is text-only unless the name says vision; Gemini, GPT-4o/4.1/5, Claude and
  the `-vl` / llava families keep seeing; an unknown model is treated as
  blind. The gate names the models on this machine that can read an image.
- **Chat: stop acts on the session** — the stop button was gated on the
  selected agent's run, so an embedded skill chat streaming on `ling` showed
  none. The composer draws no scrollbar of its own while it is still growing.
- The empty-core hint no longer sends working rules to core. Screens as
  documents Yinyue may arrange: `doc/dynamic-ui-spec.md`.

- **A skill page can tell "the agent is quiet" from "we went deaf"** — the
  embed iframe now relays its transport status to the page hosting it
  (`connection` event, alongside the existing token/content_block relay). A
  skill app has no transport of its own, so it could not distinguish the two:
  Pulse called a healthy 5-minute gather a failed run because its data
  channel spent 3 minutes reconnecting after a page reload and the events
  arrived, correctly buffered, only after its watchdog had given up.
- **`x_mentions` on the MCP front door** — an MCP client now gets the same
  mentions read the Pulse skill does: recent mentions of, and replies to, the
  signed-in user, from their x.com Mentions tab. Needs linggen-browser 0.4.7,
  where the op became real.
- **No tool observation can blow the context** — stdout and stderr from a
  command now come back capped at 64 KB per stream, head and tail kept, the
  dropped middle replaced by a marker naming how much went missing. Applies
  everywhere a process's output reaches the model: `Bash`, skill shell tools,
  app launcher scripts. Skill tools declare their own budget with
  `max_output_bytes:` beside `timeout_ms:`. A Pulse shape probe had dumped
  198 KB into one observation and ended the run with a text-only exit.
- **A talkative skill tool no longer stalls until its timeout** — skill tool
  output sat in the pipe unread while the poll loop waited for the process to
  exit, so anything writing past the ~64 KB pipe buffer blocked on the write
  and was reported as a timeout after the full `timeout_ms`. Both pipes are
  now drained on reader threads, the way `Bash` already did it. A 200 KB
  skill tool went from 30s-and-"timed out" to 0.04s. Skill commands also run
  in their own process group now, so a real timeout kills the whole pipeline
  instead of just the shell.
- **Yinyue's "reply is ready" is a notification, not a summary** — the
  run-finished herald used to quote the first 300 characters of Ling's reply,
  and she read the cut as the reply breaking off. She now gets the user's own
  question for the topic and is told she has not read the reply: one line
  that it's ready, never what it did. "Ling needs you" keeps its own AskUser
  hook.
- **`ling status` tells the truth about the daemon** — the Agent line only
  prints a PID that is alive; a listening port with a dead `ling.pid` shows
  the real listener (via lsof) and names the stale file. `ling --web` now
  writes `ling.pid` itself, so the file is right however the server was
  launched (it had been showing a pid from July).
- **Model probe mirrors the engine's routing** — every OpenAI-compatible
  provider (deepseek, gemini, groq, …) is pinged at `/models` with the key
  resolved the way the engine resolves it (TOML › credentials.json),
  Anthropic at `/v1/models`, OAuth-backed models report their sign-in
  state. No more "unknown provider" for providers the engine runs every
  day.

## [1.8.1] - 2026-09-01

### Session surfaces, fixed from one browse of the list

Five defects found by using the session list the day 1.8.0 shipped —
all root-caused, none patched around.

- **Cold load shows the transcript** — a restored session rendered empty
  on a fresh page load: the connect-time state fetch was gated on user
  permission, still `pending` when the reconnect fired, and nothing
  retried once it arrived. The first permission arrival now re-runs the
  fetch.
- **Attended mission sessions stay missions** — a session you started
  from a mission was classified as plain chat everywhere: no mission
  badge, the wrong state endpoint, and after a daemon restart a chat
  turn ran with no mission context at all. The UI classifies by
  `mission_id`, and chat turns re-bind the session's mission from meta
  the way skill sessions already do (body, allowed tools, no memory,
  prompt cache invalidated).
- **Spinners die when runs end** — runs that finish outside the chat
  path (agent chat, watcher heralds) never emitted `Idle`, so the list
  showed them busy forever. `finish_agent_run` emits `Idle` for every
  top-level run at the one choke point; subagent runs are excluded since
  they share the parent's session.
- **Catch-up waits for quiet** — background mission catch-up moved off
  the after-every-chat-turn hook onto the scheduler tick: once a minute,
  gated on a 10-minute quiet window (no user chat turn, no top-level run
  in flight), so mission work never competes with you for the model. The
  quiet clock seeds at boot, so a fresh daemon waits out its first
  window. Cron fires of catch-up-capable missions defer the same way;
  missions without `catchup_hours` still fire on schedule.
- **Session list groups by last activity** — Today / This Week / This
  Month / This Year / Older. The group, the order within it, and the
  row's relative time all key on the transcript's last write
  (`updated_at`, derived at list time, never persisted), so a two-week-old
  session touched today sits under Today.

## [1.8.0] - 2026-08-31

### The engine brings its own runtime — and a voice

1.7.0 made the phone a full peer. 1.8.0 turns inward: the engine carries
a managed Python, Yinyue speaks with a real voice, every prompt shares one
human voice layer, and a stop actually stops.

- **A managed runtime** — a pinned, relocatable CPython lives under
  `~/.linggen/runtime`, fetched and repaired on boot (SHA-256 verified,
  resumable, GitHub → linggen.dev mirror fallback) with named venvs:
  `tools` (yt-dlp nightly) and `tts`. Skill scripts get a deterministic
  interpreter via `${LINGGEN_PY:-python3}`; a Mac with no python3 at all
  now has one. Progress rides the retained `tasks` topic; low disk stops
  the prewarm cleanly until next boot.
- **Yinyue has a voice** — Qwen3-TTS (Apache-2.0) through a resident MLX
  sidecar on Apple Silicon, with Kokoro and then `say` as automatic
  fallbacks — proven by killing the sidecar mid-session. Warm synthesis
  ~650ms; a degraded GPU gives up in seconds instead of a minute, and the
  sidecar respawns in the background. `pet.voice` picks her speaker
  (default vivian) from Settings, live on her next line; her bubble stays
  up until the audio ends. The model download gates on the pet being on.
- **One voice for every agent** — `agents/shared/voice.md` is embedded
  and injected into every system prompt (user, skill, mission), and the
  full humanize catalog ships built-in under `~/.linggen/agents/shared/`.
  Agent specs resolve mdBook-style `{{#include}}`. The two "format with
  Markdown" lines that were overriding it are gone.
- **Stop stops now** — model streams poll cancel every 250ms, in-flight
  tools abort the way a timeout does, a cancelled run can never finish
  Completed, and queued messages survive the cancel and run as the next
  turns.
- **Memory doctrine from one source** — the engine injects every
  connected MCP server's `instructions`; ling-mem's are the canonical
  memory protocol, so the hand-copied `[memory_protocol]` is deleted
  (~5k tokens saved per turn). The nightly dream audit merges the
  provable, queues the rest, and digests quiet clusters (≤5 a night);
  `replace_ids` archives its losers instead of deleting them.
- **Chat can carry an image again** — client→server control messages
  are chunked (the one direction that wasn't; a 256 KB SCTP cap tore the
  channel down), and images downscale to 1568px before upload. A 29 MB
  PNG now arrives.
- **Shell children see your PATH** — a GUI-launched daemon inherited
  launchd's bare PATH; every shell lane (Bash tool, `/api/bash`, skill
  `cmd:` tools, both launchers) now appends the user bin dirs and the
  runtime, so `ling-mem` in `~/.local/bin` and Homebrew resolve however
  `ling` was started.

### Also

- Telemetry ships one daily digest row (turn/error/update counts, coarse
  error codes only, never text); the audit header enumerates every `via`
  value; the ling-mem auto-install declares itself as the channel and
  falls back to the linggen.dev mirror.
- Browser bridge: the call timeout ceiling is 180s (x targets legitimately
  runs ~2 minutes); the spec records the bridge's single write — `post`,
  reachable only from a human's click — and the two ways reads fail
  quietly.
- A pending AskUser survives not being watched — entering the session
  backfills its question; `/?session=` deep-links in normal mode.
- `keep_alive` validates as a Go duration at save, inline next to the
  field.
- Yinyue's task-finished herald quotes the run's last words and reports
  only what they support.
- `resolve_ling_mem` stops probing `/usr/local/bin`; `~/.local/bin` is
  the singleton.
- `build.rs` watches `agents/`, `missions/` and `ui/dist/` — a new file in
  an embedded folder rebuilds the crate.
- Canonical Apache-2.0 license text restored (GitHub had classified the
  repo NOASSERTION).

### Fixed

- Reject on a plan did nothing: the handlers keyed by the raw project
  root while chat stored the canonical one, recovery bailed on the first
  non-JSON message, and the RTC control channel returned a 404 as
  success. All three fixed; failures now surface as toasts.
- IME composition Enter no longer submits half-composed text in any
  Enter-to-submit input.
- A daemon killed mid-turn now says "No response — the run was
  interrupted" instead of leaving "Musing for 3m 5s" on screen.
- The keep_alive error in Settings no longer truncates.

## [1.7.0] - 2026-08-12

### The phone can find you anywhere

The release the iOS app expects. 1.6.0 taught the engine to pair on the
LAN; 1.7.0 makes the phone a full peer from anywhere.

- **Pair from anywhere** — the QR reaches past the LAN (it carries the
  relay), and the pairing window exists only while you're showing it. A
  code retires the moment a phone uses it, Revoke takes the relay
  credential back too, a guest gets a name, and `/api/pair/me` answers
  over the tunnel so a phone can relearn the relay. A new phone starts
  with two models, not one, and unpaired peers are confined where peers
  are actually judged.
- **A device is itself** — a paired phone answers with its own name (the
  same name on the LAN as over the relay) and takes the owner's
  permission ceiling, rather than being handed Admin.
- **Perception** — a resident agent can see the machine it lives in: a
  device-local activity log (`~/.linggen/activity/`, `POST /api/activity`,
  capped and attributed), a doorbell that claims nothing without a reader,
  readings that cross to the other machine, and "what happened" meaning
  both machines, not this one.
- **Relay speed** — vendored str0m + sctp-proto forks; relay transfers
  were SACK-paced and now batch-fill the SCTP send buffer; bulk media gets
  its own write lane so control frames never queue behind it; relay peers
  do full ICE and advertise a STUN-discovered public candidate.
- **Memory is scoped to the project** — a row keeps the cwd it was born
  in, recall asks for scope, a promote keeps its origin, and the phone's
  memory comes back over the tunnel.
- **MCP both ways** — ling consumes MCP servers, not only serves them;
  ling-mem ships as the built-in one, so memory has one surface (the
  engine's `memory_*` proxies are deprecated); a Settings tab shows the
  servers as they actually are.
- **Media, safely** — every synced file records which phone and account
  sent it; a phone's roll speaks only for that phone; the Mac never offers
  a phone a deletion it cannot put back; the delete queue shows itself to
  its owner and unclaimed rows drain; the file watchers are gone — the
  queue is pulled.
- **Honest model selection** — the selection layer consults auth state
  everywhere it chooses: defaults, the fallback chain, `/api/models`, and
  pickers say "sign in required" instead of offering models that cannot
  answer.

### Also

- App actions are tools: one writer per mutation, a phone tool registry,
  typed cross-device calls.
- A topic can keep its last value, for surfaces that were not listening —
  the `tasks` topic carries long-run progress both surfaces read.
- Every speaking turn wears its name — the sender rides the request, and
  every surface labels from it.
- Skills declare their own renames and keep their paths;
  `/api/skills?brief` drops the SKILL.md bodies; a phone on the tunnel is
  still a phone, and devices say whether they are here right now.
- Yinyue answers app questions from the canonical guide, and can open the
  door her own prompt points at.

### Fixed

- A dismissed queued message stays dismissed — the queue map is the
  authority.
- A missing directory silently disarmed every media watcher (before the
  watchers left entirely).
- The same question is no longer asked once per message, and two promises
  the code was not keeping are kept.

## [1.6.0] - 2026-07-27

### The engine grew a phone

Everything Linggen Mobile needs to be a real second device rather than a
viewer. The bulk of this release is one arc.

- **Pairing** — QR pairing with identity preview, a LAN gate (per-device
  tokens + a screen-confirm handshake) so strangers on your Wi-Fi can't
  reach the daemon, Bonjour advertisement on LAN binds, device dedup by a
  stable id, inline rename, and a Settings → Phone tab to run it all.
  `GET /api/pair/me` resolves a per-device model catalog, so a phone offers
  exactly the models that Mac allows it.
- **Media sync** — `/api/media/*`: wireless Photos backup, manifests
  carrying Apple Shifu scan verdicts, mirror-vs-archive split (reconcile +
  backup), a reversible Mac→phone delete queue, and an auto-run of the
  Media pipeline scan once an ingest quiesces.
- **Transport** — a binary media channel moving bulk bytes at ~30 MB/s
  instead of 0.4, device topics so the Mac announces and the phone stops
  polling, and the relay address handed to the phone so all of it works off
  the LAN.
- **DJ** — per-device sync ledger, library routes, karaoke instrumental and
  video exposed to the phone.

### Also

- **One account credential.** `~/.linggen/account.toml` covers billing,
  cloud models, relay and rooms; the second stale copy is gone. Signed in
  means online. `/api/account/mobile-token` was removed — a paired phone no
  longer adopts this Mac's account token.
- **Device sync is declarative** — skills describe what to sync in their
  frontmatter; no app-named Rust in the engine.
- **`app.list: true|false`** — a skill can declare itself out of the
  launcher's tab bar while staying installed and runnable.
- **Web search is login-only cloud** — no per-user Tavily key.
- **`/api/health` reports the engine version.**

### Fixed

- **One dead peer could take the whole daemon down.** A WebRTC peer whose
  DTLS had failed asked for a timeout already in the past, on a branch with
  no `.await` — so it pinned a tokio worker at 100% and starved the runtime.
  The daemon stopped answering everything, including `/api/health`. Now the
  branch yields, and a peer that makes no progress for 5s is torn down.
- A dead channel can't strand the UI, and no tool owns a run forever.
- A half-received transfer is dropped when the peer goes away.
- Yinyue speaks Chinese again — the voice was hardcoded to English.

## [1.5.0] – [1.5.1] - 2026-07-17

Dual port migration: the engine moved to **9527** (ling-mem to 9528), with
`LINGGEN_PORT` to override. `memory_dream_status` passes counts through.

## [1.4.0] - 2026-07-10

### MCP front door — use Linggen from any agent

- **`/mcp` endpoint** — the daemon now hosts a streamable-HTTP MCP server at
  `http://127.0.0.1:9898/mcp` (serverInfo `linggen`): 22 tools in four
  groups. `browser_*` (10) drive one visible tab in your own Chrome through
  the linggen-browser extension; `x_*` (5) return structured reads from your
  logged-in X session; `memory_*` (6) proxy the ling-mem store; `agent_run`
  delegates a task to a local Linggen agent (read/memory/browser toolset)
  and returns its answer.
- **The `linggen` plugin/skill replaces `shared-memory`** — Claude Code and
  Codex connect via `linggen@linggen-memory`; OpenClaw via
  `clawhub install linggen`; any agent via skills.sh. The plugin installs
  both required binaries on first session start (ling-mem immediately, the
  engine as a disclosed background download).
- **Browser gate lives in the extension** — per-origin permission prompt
  with persistent trust, one gate for every caller (Linggen agents,
  MCP clients); the engine-side gate is retired.

### Dream

- **Condense folded into dream** — the nightly pass ends with a cited-chains
  merge stage (derived-only, capped); the standalone condense mission is
  deleted. The engine snapshots the store before each run (7 kept).
- **Attended dream runs** — clicking a calendar day starts an attended run
  that asks about low-confidence merges in one batch; AskUser is now
  chat-tier so missions can use it.

### Models

- **ChatGPT built-in is `gpt-5.6-terra`** — replaces gpt-5.5, with
  retired-id config migration; the built-in entry always wins over a stale
  user config row. Fresh installs seed the Linggen Cloud default model.

### Fixed

- **ling-mem installer URL** — the runtime bootstrap pointed at the old
  `plugins/shared-memory` path, which 404s after the plugin rename.
- **Yinyue heralds** — bubbles linger 15–45s, survive tab switches, and
  count down only while the tab is visible.
- **Missions** — runs recorded up front and healed on start; sticky RTC
  view context; 60s outer timeout on the memory-tool dispatch path.
- **UI** — pointer cursor restored on all buttons; launcher gains a
  "Start at login" toggle.

## [1.3.0] - 2026-07-08

### Memory — the merge law reaches every host

- **User-voice merge guard at the store** — silent replaces or rewrites of
  rows the user authored are mechanically blocked unless the write carries
  `user_directed: true`; the engine forwards the flag end-to-end.
- **Condense** — monthly mission collapsing stale same-subject chains in
  long-term memory (v1), plus a subject-digest phase (v2); cron enabled.
- **Dream / missions** — day-scoped triggers (`kickoff-day`), kickoff
  early-drain on completion sentinels, engine-composed run reports, no
  orphan sessions.
- **Core block over HTTP** — session-start core memory reads the daemon
  directly instead of shelling out to the CLI; live writes fill
  `source_session`; relative time anchored at capture.
- **Write-side memory eval** — 20-scenario dataset with runner + checker.

## [1.2.3] - 2026-07-02

- **Account avatar in both shells** — the launcher header and the dev console
  now show the signed-in billing account (avatar + name, Dashboard, Account
  settings, Sign out) so it's always visible which account the daemon spends
  from. The dev console previously showed the remote-relay identity, which
  fails on a local daemon.
- **Dream mission hardening** — survive already-deleted rows, cross-table
  dedup search, 25-row worklist pages with a re-list loop, provenance notes;
  no delete after promote.
- **Compaction** — token usage re-measured at the trigger point.
- **In-page dialogs** — native `confirm()`/`prompt()` replaced with in-page
  dialogs (no-ops inside app shells).
- **Model routing** — fallback error classifiers match real provider status
  formats; cancel recorded as cancelled; reconnect model fallback.

## [1.2.2] - 2026-06-30

### Linggen Launcher — one shell for every app

The web UI is now a native app-host: a single Linggen shell that hosts the
products and skill apps side by side, instead of one app per window.

- **Header app menu** — switch between apps from a menu in the launcher header.
  Local skill apps open as tabs in the same shell rather than spawning new
  windows, and the menu is ordered products-first.
- **Merged settings** — a single ⚙ in the launcher header opens unified Linggen
  settings (account sign-in plus the Yinyue panel with a model picker), reached
  from the shell's native Settings menu.
- **App-aware routing** — `agent_chat` takes an optional `app` target to route a
  message into that app's session, and Yinyue routes apps by the live
  available-skills list. Skills carry `is_app` metadata and are marked "(app)"
  in the prompt list; hosted app iframes receive `in_launcher=1`.

## [1.2.1] - 2026-06-25

### Fixed

- **Self-updater on slow links** — `ling update` replaced its single 60s overall
  request timeout (which killed the ~70 MB asset download mid-transfer on a slow
  connection) with separate connect and read/stall timeouts, so a slow-but-
  progressing download of any size now completes.
- **Yinyue presenter across multiple surfaces** — with several tabs or apps open,
  Yinyue now renders and speaks in exactly one place. The server arbitrates a
  first-come-first-served presenter lock and unicasts speech/gestures to the
  holder instead of broadcasting, and promotes the next surface when the holder
  disconnects.

## [1.2.0] - 2026-06-24

### Yinyue — the desktop companion

A VRM avatar and conversational companion built into the runtime. She lives at
the edge of the web UI, reads the room, speaks in her own voice, and gives the
agents a face and a way to talk to each other. She runs as an ordinary Linggen
session on the `yinyue` agent — same engine, persistence, and shared memory as
any agent, with a narrow tool set and a spoken output sink. Design in
`doc/yinyue-companion-spec.md`.

- **Presence sensing** (`sense` tool) — she reads whether you're *typing*,
  *reading*, or *away* from a privacy-light client beat (`POST /api/presence`:
  recency + focus only, never keystrokes). Reactions are gated on it — quiet
  while you're heads-down, a word when you've stepped away.
- **Agent heralds** — her watch loop voices the moments that matter, in her own
  words: an agent blocked on you (question or permission), a run that failed, a
  background job finished. Silent on the routine.
- **Relay an answer** (`answer_prompt` tool) — when an agent is parked on a
  prompt, approve or answer it by telling Yinyue; she carries your word back and
  the agent unblocks. She relays, never decides.
- **Ambient life-signs** — a server-side jittered loop where she glances at the
  day and, now and then, says one small unprompted thing. Mostly silent.
- **`agent_chat`** — general one-way inter-agent messaging; any agent can message
  any other. A message to a chat agent lands in its chat as `[Sender]: …` and the
  agent responds there (routed to the session you're viewing); a message to
  Yinyue she acts on (speaks or moves). A one-hop loop-break (a turn reached via
  `agent_chat` can't emit another) keeps the user in the loop.
- **`Express`** — avatar body control: a sustained mood + one-shot gestures (nod,
  wave, dance, …). **Pet-scoped** — granted only to an agent that lists it, so a
  worker agent on `tools: ["*"]` can't drive the avatar; it asks Yinyue via
  `agent_chat` instead.
- **Local voice** — she speaks via on-device TTS; bubble + lip-sync track the
  audio.

### Added

- `sense`, `answer_prompt`, `agent_chat` built-in tools; `Express` advertised to
  the companion.
- Per-agent presence (`POST /api/presence`) and a focused-session signal
  (`set_view_context`) so `agent_chat` delivers into the chat you're viewing.
- Pet-scoped tool grants — a tool may be excluded from the `*` wildcard and
  granted only to agents that list it explicitly.

### Fixed

- Avatar rest pose — arms hang in a natural A-pose on VRM1 rigs (the upper-arm
  roll is mirrored vs VRM0).

## [1.1.2] - 2026-06-17

### Added

- Per-app scoped memory — a skill can declare `memory-context` (plus
  `memory-recall-min-score` and `memory-recall-count`) in its frontmatter so
  the agent reads and writes memory only within that namespace. Auto-recall and
  the `Memory_*` tools are scoped to the skill's context, keeping a focused app
  (e.g. CFO) from pulling in unrelated cross-app memories.
- ling-mem is pre-warmed at startup when any installed skill uses scoped
  memory, so the first recall has no cold-start delay.

## [1.1.1] - 2026-06-15

### Added

- Per-app usage attribution — outbound model calls carry an `X-Linggen-App`
  header so the linggen.dev proxy meters tokens per (account, app), enabling
  per-app trials and subscriptions for branded apps.
- Settings: configurable memory recall count (default 3) and recall
  similarity threshold (default 0.7).

## [1.1.0] - 2026-06-11

### Account & Linggen Cloud

- **`ling account login`** + daemon `/api/account*` endpoints — browser
  sign-in to the linggen.dev account, token stored in
  `~/.linggen/account.toml` (0600, separate from remote access). Entitlement
  and free-trial state are served with a short cache and offline grace;
  subscription checkout is proxied for app shells.
- **Built-in Linggen Cloud model.** `deepseek-v4-flash` ships in every
  install, routed through linggen.dev with the account token resolved per
  request — no API key. A user-defined model with the same id always wins.
- Built-in models appear under Settings → Models → Built-in and can be set
  as the default.
- Payment-required responses surface the subscribe / trial message verbatim
  instead of a raw provider error.

### Fixes

- OpenAI strict-mode tool schemas + Responses-API reasoning effort; more
  empty-response retries for reasoning models.
- WebRTC: large unsolicited pushes are chunked so `page_state` can't reset
  the data channel.
- Auto-compaction is surfaced in chat; permission gating for
  upward-relative bash args; memory capture nudges and a lower episodic
  gate; skill-page-injected assistant messages render in the embed.

## [1.0.0] - 2026-06-02

First stable release. The public contracts — the skill / agent / mission
spec formats, the `~/.linggen` storage layout, `linggen.toml`, and the
install path — are now semver-stable: backward-compatible changes ship as
minors, breaking changes as a major.

### Memory — inline capture (Complementary Learning Systems)

- **Inline, per-turn capture.** The live `ling` agent writes memory as it
  goes — `core` / `semantic` / `episodic` via the built-in `Memory_*`
  tools, routed by salience and confidence. This **retires the previous
  every-N-turns encoder subagent**; capture is no longer a separate
  wake/sleep pass.
- **`dream` mission as audit + consolidator.** A built-in mission (runs
  as `ling` on a daily cron plus a turn-seam catch-up) re-reads recent
  sessions to catch anything the live agent missed, then consolidates
  past-TTL `episodic` rows (promote → `semantic`/`core`, or evict).
  Per-run stoppable/deletable; each run leaves an audit record.
- **Read-before-write reconcile contract.** Every write is checked
  against existing memory: near-duplicates deduped mechanically, genuine
  contradictions resolved **with the user** (dated, appended — never
  silently overwritten). Store stays CRUD-only. Full contract in
  `doc/memory-spec.md` §2.
- `Memory_query` / `Memory_write` are first-class **built-in tools**
  (Chat-tier, ungated) — the capability layer is gone.
- **Per-turn auto-recall** with score-gated injection
  (`agent.memory_inject_min_score`, default 0.6) and visible
  "From memory: …" citations.
- The `ling-mem` binary is **semver-range-pinned** (`~0.8`) and
  **auto-installed / auto-started** when missing; the store's
  schema-version guard (`linggen-memory/doc/schema-versioning.md`) makes
  in-place subversion upgrades data-safe.

### Engine architecture

- **engine / extensions split** — skill, agent, and mission share one
  record + registry shape in `engine/`; disk loaders live in
  `extensions/`.
- **Tool trait + builtin registry** — all tools are async and
  schema-driven; removed the capability layer and the dual sync/async
  bridges. Large `mod.rs` files thinned; module boundaries reorganized.

### Permissions

- `path_modes`-based model; reads are gated like writes; a hardcoded
  curated deny floor backstops catastrophic commands.
- A skill's own provided tools bypass `allowed-tools` gating; declared
  grants apply **silently** (no activation prompt); the OS temp dir is
  always-allowed scratch; quote-aware path extraction.

### Chat & providers

- **ChatGPT OAuth** — silent refresh on 401, and an inline **"Sign in
  with ChatGPT"** CTA when the session truly expires, on every turn path.
- Fail-fast on missing/expired OAuth with a clear status.

### Compaction

- CC-aligned **two-tier** compaction with per-session config; `/compact`
  fixed on tool-heavy sessions.

### Missions

- `dream` is a real built-in mission; raw-markdown mission editor with
  frontmatter-safe live preview; per-mission `catchup_hours`.

### Other

- Telemetry module — install + command events only, daily-deduped
  client-side; never sends prompts, responses, file contents, or paths.
- Sessions: auto-rename placeholder titles from the first user message;
  spinner driven by `busy_sessions` / agent runs.
- Dependency bumps (str0m 0.16→0.19, grep 0.3→0.4, cron 0.15→0.16).

## v0.10.0 (2026-04-29)

Proxy rooms, user isolation, mission as first-class subsystem, memory system redesign, and LAN access fixes.

### Added

**Proxy rooms (Phase 6a + 6b)** — share AI model access with others through invite-only or public rooms.

- **Private/public rooms** with shared models, allowed tools, and allowed skills configured in `~/.linggen/room_config.toml`.
- **Two consumer modes** — browser-based (linggen.dev/app) and linggen-server (outbound WebRTC client connects via relay signaling).
- **Proxy provider** — `ProviderClient::Proxy` routes inference over the WebRTC inference data channel and streams `StreamChunk`s back to the consumer.
- **Inference data channel** — separated from the control channel; `list_models` + `inference` handlers run there, filtered by `shared_models`.
- **Settings → Sharing tab** — room management, shared model checkboxes, allowed tools/skills, member list.
- **Persistent token budget** — `~/.linggen/token_usage.json` store with room-level (`token_budget_room_daily`) and per-consumer (`token_budget_consumer_daily`) daily limits. Auto-resets at midnight UTC, flushes every 30s, survives reconnects.
- **Token usage UI** — Room tab reads usage and budgets from local store; bar updates in real time.
- **Auto-refresh model list** on proxy connect/disconnect via `StateUpdated` event.
- **Room chat** — bidirectional chat over the inference channel with `sender_id` echo prevention. User profile (`user_id`, `user_name`, `avatar_url`) backfilled from relay on startup; falls back to `instance_name` when `user_name` is missing.
- **Disconnect on disable** — toggling a room off broadcasts `RoomDisabled`, kicks consumer peers, syncs status to the linggen.dev DB. UI shows amber "Disabled" badge and hides Connect.
- **Model selector labels** show the room name — `proxy:gpt-5.4 (My Room)` instead of generic owner attribution.

**User isolation**

- **`UserContext`** replaces `ConsumerContext` — every peer carries `user_id` + permission level. `room_name` flows through signaling → `UserContext` → page_state.
- **Unified `page_state`** filtered per-user (sessions, models, skills, busy_sessions) — no more separate consumer page state.
- **`SessionMeta.user_id`** tracks ownership; `ChatRequest` accepts `user_id`, injected by `peer.rs`.
- **`ConsumerFilter`** drops events for other users' sessions on outbound delivery.
- **`user_id` persisted** in `remote.toml` (returned by linggen.dev registration).
- **Hidden messages** — `[HIDDEN]` content is filtered from all session-state APIs (workspace, skill, missions) before sanitization, and from `ChatPanel` rendering. HTML comments (`<!-- ... -->`) are stripped from rendered markdown. New `send_hidden` action in the skill bridge for system-level prompts.

**Mission as first-class subsystem** — missions are no longer a "mission skill"; they are a sibling subsystem with skill-shaped markdown.

- **Frontmatter matches `SKILL.md`** — nested `permission { mode, paths, warning }`, `allowed-tools`, `allow-skills`, `requires`, optional `entry` script.
- **Three modes** — `agent` (default; create session + run agent loop), `app` (open `entry` URL in browser), `script` (run `entry` as shell command). App/script modes skip the agent loop entirely.
- **Entry script** — runs before the agent loop, captures stdout/stderr to a per-run output dir, passes `MISSION_*` env vars. `~` in cwd is expanded before spawning, fixing the ENOENT that blocked Bash calls.
- **Tool scope** — `allowed-tools` + `allow-skills` drive `mission_allowed_tools` and `consumer_allowed_skills`. `*` means any skill; an empty `allow-skills` removes `Skill` from the allowlist; a concrete list gates the `Skill` tool even when `allowed-tools` is empty.
- **Mission body** injected into the system prompt via `active_mission` (mirrors `active_skill`), not as a 3 KB user message. The user turn is a short kickoff. `get_system_prompt_api` restores `active_mission` so copy-prompt returns the real prompt for mission sessions.
- **In-memory mission cache** — loaded once at startup, refreshed on create/update/delete and after skill install. Scheduler reads from cache (zero disk I/O per tick).
- **`MissionEditor`** rewritten — policy (4-way), permission mode + paths + warning, allowed-tools, allow-skills, requires, entry, description. `MissionNav` simplified; mission sessions live in the main session list under the Mission tab.
- **API** — `POST/PUT /api/missions` accepts new fields and legacy aliases (rewritten on next save). `GET /api/missions/{id}/runs/{run_id}/output` returns entry-stage stdout/stderr.
- **Bundled "dream" mission** rewritten in the new format with a real entry script.

**Memory system redesign**

- **Skill `install` field** — runs a script on install, wired into all install paths (marketplace, built-in, init). Replaces `SkillMission` / `create_mission_for_skill` — missions are now asset files copied by install scripts, not engine-managed.
- **Memory frontmatter is fixed** — templates are the source of truth; the model only edits the body.
- **`Edit` added** to the memory tool allowlist (alongside `Read`/`Write`).
- **`memory_descriptions_block_empty`** bootstrap template for first-run when no memory files exist. Old `memory_block` / `global_memory_block` templates removed.
- **`$SKILL_DIR`** now resolves correctly in skill content injection (prompt.rs + delegation.rs).

**UI architecture**

- **App entry restructured** — `App.tsx` / `main.tsx` replaced by `apps/{Main,Consumer,Embed}App.tsx` and `entries/main.tsx`, one bundle per surface.
- **Event dispatcher split** — per-kind handlers under `eventHandlers/`, table-driven dispatcher. Canonical `EVENT_KINDS` list; `UiEvent.kind` is now an exhaustive union.
- **Store refactor** — `projectStore` → `sessionStore`, `agentStore` → `serverStore`; new `userStore` and `interactionStore`.
- **Session list** reordered to User / Mission / Skill / All with per-tab counts; defaults to User. Trigger now sends a proper JSON body.
- **Chat actions** — Copy Chat / Copy System Prompt / Clear Chat moved out of `HeaderBar` into the expandable session bar inside `ChatPanel`, so every chat surface (main, skill, embed, consumer) has them.
- **System prompt export** — frontend sends `session_id` so skill-bound sessions export the active `SKILL.md` body; backend falls back to session cwd when `project_root` is empty; native tool schemas included.
- **Skills sorted by usage** — localStorage click count + last-used timestamp.
- **`HeaderBar`** shows the room name (clickable → Settings → Sharing). Leave button navigates to linggen.dev/app.
- **Subagent return messages** render as their own chat bubbles in live-stream sessions, matching the persisted `messages.jsonl` replay path.

**LAN / WebRTC**

- **WebRTC binds to `0.0.0.0`** and advertises the real LAN IP for ICE — fixes failures when the browser connects via LAN IP instead of localhost.
- **`--host` passed to daemon** — was ignored by the daemon child process before. Startup message now shows the actual host.
- **Auth login uses the real host** — was hardcoded to localhost, blocking login from LAN IPs.
- **Skip browser open on headless Linux** — when neither `DISPLAY` nor `WAYLAND_DISPLAY` is set (SSH sessions).
- **ChatGPT OAuth tokens read from disk per-request** — login/refresh takes effect immediately without restart. `ModelManager` rebuilt after OAuth completes.

### Changed

- **Tool restriction model** — single source of truth via `EngineConfig.effective_tool_restrictions()` (cascading mission ∩ consumer intersections); `EngineConfig.is_tool_allowed()` is the unified check used by both prompt building and the execution gate. Two separate gates (mission + consumer) collapsed into one in `tool_exec.rs`. `consumer_allowed_tools` normalized to `HashSet`.
- **`ModelInfo.provided_by`** field added so the UI can attribute proxy models.
- **Increased proxy answer poll timeout** — 30s → 60s to tolerate slow relay delivery before the owner picks up the offer.
- **Filter messages input dropped** — the browser's Cmd+F is sufficient.

### Removed

- **`AgentTree` component** (dead code).
- **`/api/projects` endpoints** and `projects` from `page_state`. `MissionPage` derives working folders from sessions; project-related store methods deleted.
- **`ProjectInfo` type** and related project store methods.
- **The "mission" skill** — missions depend on skills via `requires:`, not on a gated skill. Scheduler and API no longer check for it.

### Fixed

- **Permission mode UI not updating after approval** — `page_state` handler referenced an undefined `_permissionSuppressedUntil`, so `Date.now() >= undefined` always evaluated false and blocked all updates. The UI showed stale "read" mode even after the user approved a switch to "edit".
- **Auto-cleanup proxy connection on disconnect** — consumer auto-removes the stale connection, Room tab shows "Connect" instead of stuck "Connected".
- **Room chat panel hidden** when the owner had room enabled but no `user_name` yet.
- **`list_models` race** — wait for the inference channel to open before sending; double-clickable Connect button now shows a spinner and is guarded.
- **Skills installed flag stale** — Library card kept showing "Install" for freshly-installed built-in skills until the 10-min cache TTL expired. Cache only the GitHub-derived metadata (dir_name/name/description); recompute the `installed` flag from the filesystem on every call.
- **File watcher removed** — crashed on permission-denied paths; wasn't used.

## v0.9.3 (2026-04-08)

Server-pushed PageState, TUI removal, auto-scroll rework, and UX polish.

### Added

- **Server-pushed PageState** — server aggregates projects, sessions, models, skills, agents, missions, and permissions into a single message pushed over the WebRTC control channel at 0.5 Hz with a dirty-flag mechanism. Replaces the HTTP polling storm that fired on every agent run.
- **`set_view_context` message** — frontend tells the server which session/project is active, scoping PageState pushes to relevant data only.
- **`busy_sessions` in PageState** — remote clients now see session busy status without needing per-session event channels.
- **Dismiss button on queued messages banner** — manually clear the queue when it gets stuck (e.g. `QueueUpdated` event missed).
- **Markdown links open in new tab** — `target="_blank"` on rendered links so clicking doesn't navigate away and text selection is easier.

### Changed

- **Auto-scroll rework** — replaced distance-threshold detection with scroll-direction detection. Added `distanceFromBottom > 150` guard so layout reflows during streaming no longer falsely detach auto-scroll. Consolidated duplicate scroll tracking from ChatPanel into the single `useAutoScroll` hook.
- **Removed HTTP polling** — initial load fetches for projects, sessions, models, skills, agents, and config are all replaced by PageState delivery on WebRTC connect. Only Ollama status and session tokens remain as HTTP fetches.
- **Removed 5 dead API endpoints** — `agent-children`, `agent-context`, `missions/:id GET`, `missions/:id/sessions`, `builtin-skills/install-all`.
- **SessionModeSelector simplified** — reads mode and zone from store (pushed by PageState) instead of fetching `/api/sessions/permission` on every render.
- **Extracted non-reactive agent tracking** — `agentTracker.ts` singleton replaces 15+ direct Zustand store mutations in `eventDispatcher`.
- **Memoized skill suggestions** in ChatInput (was rebuilt twice per render).
- **Tokens/sec display wired up** — `recordTokenSample` + `recomputeTokenRate` were never called; now functional.

### Removed

- **TUI** — terminal UI (ratatui) and `--tui` flag removed. Linggen is now Web UI only. `ling` starts the daemon and opens the browser; `ling --web` runs the server in foreground.
- **SSE transport** — server-sent events transport removed. All real-time communication uses WebRTC data channels.

### Fixed

- **Auto-scroll fighting** — removed duplicate scroll tracker in ChatPanel that competed with `useAutoScroll` hook.
- **Session mode selector race** — after user switches mode (e.g. admin → read), a 3-second suppress window prevents the next PageState push from overwriting the optimistic UI update.
- **Page flash during streaming** — `floatingUserMsg` effect was re-subscribing on every token.
- **Duplicate React key** in SubagentTreeView.
- **Subagent state leak** — `agentTracker.reset()` called on session switch.
- **Plan message overwrite** — `mutateLast` guard prevents fast-path from overwriting plan messages with streaming tokens.
- **Skill session chat in remote mode** — skill app chat iframe now routes through the relay connect page when accessed via linggen.dev (was loading the landing page instead of the compact chat).
- **Skill session restore** — reopening an existing skill session with no localStorage cache auto-triggers a fresh scan instead of showing an empty dashboard.

## v0.9.1 (2026-03-31)

Simplified run system, daemon mode, ChatGPT default model, and bug fixes.

### Added

- **Background daemon mode** — bare `ling` now spawns a background daemon and opens the Web UI in the browser. Terminal returns immediately. Use `ling --tui` for classic TUI mode.
- **ChatGPT OAuth default** — new installs default to GPT-5.4 via ChatGPT subscription. No API key or local model download needed.
- **Unified working folder** — all tools (Read, Write, Edit, Glob, Grep) resolve relative paths from the agent's cwd, not just Bash. When the agent `cd`s into a git repo, the workspace root, CLAUDE.md, and permissions update automatically.
- **User `! cd` tracking** — `! cd /path` in the Web UI now persists cwd per session, same as agent Bash commands.
- **UI follows cwd changes** — `selectedProjectRoot` updates when the agent changes working folder.

### Changed

- **In-memory run store** — agent run records are no longer persisted to `{run_id}.json` files on disk. Runs are tracked in memory only (for cancellation and status during execution).
- **Removed run history UI** — run picker dropdowns, context display, timeline, pin/unpin removed from ChatPanel and SubagentDrawer.
- **Removed dead code** — `AgentsCard.tsx` (never imported), `timeline.ts`, run context types (`AgentRunSummary`, `AgentRunContextResponse`, etc.).
- **Simplified cancel response** — `POST /api/agent-cancel` returns `{ status: "ok" }` instead of `{ cancelled_run_ids: [...] }`.
- **Font size +1px** — all UI font sizes bumped by 1px for mobile readability.
- **Logo** — shortened to "Linggen", links to linggen.dev.
- **install.sh** — removed `--with-memory` flag and ling-mem install block.

### Fixed

- **Plan reject buttons not disappearing** — `PlanUpdate` events now carry `session_id`, so they're delivered via WebRTC data channels (was `None`, events were lost).
- **Queued messages stuck after cancel** — `cancel_agent_run()` now drains the queue for cancelled agents.
- **Queued messages showing in chat** — queued messages are no longer persisted to `messages.jsonl` at queue time. They're persisted when dequeued, preventing the sync-back from re-adding them.
- **Session header not showing for user sessions** — `fetchSessions` was resetting `activeSessionId` when the session wasn't in the project-filtered list. Now checks `allSessions` before resetting.
- **Working folder in non-git dirs** — `check_working_folder_change()` now uses the cwd as workspace root when no git repo is found (was falling back to `~`).
- **macOS `/tmp` symlink** — cwd is canonicalized before use as workspace root (resolves `/tmp` → `/private/tmp`).

## v0.9.0 (2026-03-30)

Working folder model, per-session engines, WebRTC-first transport, and UX improvements.

### Added

- **Working folder model** — sessions start in HOME mode and auto-detect projects when the agent `cd`s into a git repo. CLAUDE.md, permissions, and git context load dynamically on project entry. Configurable `home_path` in settings.
- **Per-session agent engines** — each session gets its own engine instance. No more lock contention between sessions — game-table and regular chat run truly in parallel.
- **WebRTC-first transport** — Web UI always uses WebRTC (local and remote). Per-session data channels provide natural isolation. SSE retained for TUI only.
- **WebRTC session_id enrichment** — events are tagged with session_id before routing to data channels, preventing cross-session event leaks.
- **ChatGPT token expiry UX** — inline re-login button when ChatGPT OAuth expires. After re-login, session engines are cleared so the fresh token is used immediately.
- **Working folder changed event** — `WorkingFolderChanged` server event emitted when the agent `cd`s. UI header updates reactively.
- **`home_path` config** — configurable default working folder for new sessions (defaults to `~`).
- **Git root detection** — `find_git_root()` walks up from cwd looking for `.git/`. Skips home directory dotfiles repos.

### Changed

- **Flat session storage** — all sessions stored in `~/.linggen/sessions/` (flat directory). No more per-project/mission/skill session directories. Session metadata tracks `cwd`, `project`, `project_name`, `mission_id`.
- **Simplified chat creation** — clicking `+` immediately creates a session. Removed project picker dialog.
- **Removed project management UI** — no more workspace section, project cards, or manual project add/remove in sidebar. Projects are auto-discovered from git repos.
- **Skill search ordering** — community skills from skills.sh and ClawHub are interleaved by relevance instead of sorted by install count.
- **ClawHub ZIP install** — handles root-level SKILL.md (no subdirectory) in ClawHub ZIP archives.
- **Ollama status polling** — only polls when Ollama models are configured, eliminating 404 spam.
- **Auto-scroll** — any upward scroll stops auto-scroll (was 10% threshold). Resumes within 20px of bottom.
- **IME composition** — Enter key during Chinese/Japanese input composition no longer triggers send.
- **Models card scroll** — auto-scrolls to default (starred) model when the model list loads.
- **Session list** — session rows use `<div>` instead of nested `<button>` (fixes React DOM nesting warning).
- **Skill reload** — installing/uninstalling skills clears session engines so new skills are available on next message.
- **install.sh** — post-install output now shows `ling init` as the first step.

### Fixed

- **Session isolation** — WebRTC events no longer leak between sessions. Added session_id enrichment in WebRTC peer handler (was missing, only SSE had it).
- **`emit_outcome_event`** — plan/outcome events now carry session_id (was hardcoded `None`).
- **Compact mode race** — skill app iframe now explicitly fetches workspace state after setting `isSkillSession`, preventing stale API calls.
- **Session engine memory leak** — `remove_session_engine` called on all session deletion paths.
- **TUI session creation** — `get_session_meta` check uses `Ok(Some(_))` instead of `is_ok()` (was always true).
- **`UiEvent.kind` type** — added `'working_folder'` to TypeScript union type.

### Removed

- **`~/.linggen/projects/` session directories** — sessions no longer stored per-project.
- **`session_root` on `EngineConfig`** — removed; all persistence goes through global sessions.
- **`ProjectContext.sessions`** — removed; all session access through `AgentManager.global_sessions`.
- **`ProjectStore::session_store()`** — removed dead code.
- **`missions_sessions_dir()` / `skill_sessions_dir()`** — removed from `paths.rs`.
- **`NewChatDialog` component** — removed project picker dialog from UI.

## v0.8.0 (2026-03-25)

Remote access, mobile UI, Google login, and infrastructure improvements.

### Added

- **Remote access** — access your linggen from any device. Run `ling login` to link to your linggen.dev account, then connect from any browser at `linggen.dev/app`. Peer-to-peer connection — no VPN or port forwarding needed.
- **`ling login` / `ling logout` / `ling status`** — CLI commands for managing remote access. Fully automatic browser-based OAuth flow with token exchange; no manual steps needed.
- **`ling auth login`** — ChatGPT subscription auth. Auto-detects headless/SSH environments and falls back to device code flow (removed `--device` flag).
- **Google login** — sign in to linggen.dev with Google or GitHub. Email-based account matching across providers.
- **Signaling relay** — lightweight relay on linggen.dev handles connection setup. Nonce-based offer/answer exchange via stateless HTTP.
- **Mobile UI** — responsive layout auto-detected on narrow viewports (or via `?mode=mobile`). Full-bleed chat, larger touch targets, iOS safe area support. Right-side drawer for models and skills.
- **Gzip chunked transfer** — large responses (skill files, API data) are gzip-compressed and sent as base64 chunks over data channels. Handles SCTP backpressure correctly.
- **Skills open in-app** — web launcher skills now open in an in-page iframe panel instead of a new browser tab. Works in both local and remote mode.
- **Session project names for missions** — mission sessions now show their project name in the session header, matching the behavior of user sessions.

### Changed

- **`ling login` non-interactive** — uses hostname automatically, no instance name prompt.
- **Heartbeat interval** — increased from 30s to 5 minutes to reduce relay load. Online threshold set to 10 minutes.
- **Online status via D1** — instance online/offline status is now determined by `updated_at` timestamp in D1 database instead of KV TTL keys. Eliminates KV write quota consumption from heartbeats.
- **JWT sessions** — linggen.dev authentication switched from KV-stored sessions to signed JWT cookies (HMAC-SHA256). Eliminates KV reads on every authenticated request.
- **Settings page mobile layout** — scrollable tab strip, responsive model card grid, reduced padding on small screens.
- **Header compact mode** — shorter title ("Linggen" on mobile), status dot without text label, sparkles button for info drawer.
- **Session delete on mobile** — trash button always visible on touch devices (was hover-only).
- **InfoPanel component** — extracted models + skills cards into shared component used by desktop sidebar and mobile drawer.

### Fixed

- **SSRF bypass** — URL-decode path before validation in WebRTC HTTP proxy (blocks `%2e%2e` traversal).
- **JWT algorithm validation** — verify `alg: HS256` in token header before signature check.
- **Free-tier instance limit** — use `COUNT(*)` query instead of single-row check (prevents bypass via new instance IDs).
- **Token panic** — guard `api_token` length before slicing in `ling status` (no crash on corrupted config).
- **Double reconnect** — guard `handleDisconnect` against firing multiple times from concurrent ICE/connection state changes.
- **Double connect** — guard `doConnect` against concurrent calls (prevents RTCPeerConnection leak).
- **Session channel leak** — `unsubscribeSession` now called on session change in `useTransport` hook.
- **Token lost on write error** — browser response write in `ling login` callback no longer discards the received token if the browser closes early.
- **Relay poll blocking** — `handle_remote_offer` spawned in separate task so the offer poll loop stays responsive.
- **Nonce URL encoding** — relay signaling nonce is now URL-encoded in poll requests.
- **Relay offer missing Content-Type** — added `Content-Type: application/sdp` to relay offer POST.
- **Logout CORS headers** — logout response now includes CORS headers for cross-origin requests.
- **Dead code** — removed unused `split_utf8_safe` function.
- **Stale comments** — updated heartbeat interval comments from "30s" to "5 minutes".

## v0.7.0 (2026-03-11)

Major release with native tool calling, mission system, TUI, permissions, and extensive UI improvements.

### Added

- **Native tool calling** — models use structured function calling (OpenAI, Ollama) instead of JSON-in-text. Default for all providers; falls back gracefully for legacy models.
- **TUI interface** — full terminal UI via ratatui. Default mode runs TUI + embedded server; `--web` for web-only.
- **Mission system** — agents self-initiate work on cron schedules when a mission is active. Idle scheduler prompts agents between user messages.
- **Plan mode** — agents can enter plan mode (`EnterPlanMode`) for research and structured planning before making changes. Plans require user approval via `ExitPlanMode`.
- **File-scoped permissions** — `AcceptEdits` mode, deny rules, and per-project permission persistence.
- **Credential storage** — secure API key management via `/api/credentials` endpoint.
- **Model auto-fallback** — health tracking with automatic fallback to next model in the routing chain on errors or rate limits.
- **AskUser bridge** — agents can ask structured questions mid-run with options and multi-select.
- **Web search & fetch** — `WebSearch` (DuckDuckGo) and `WebFetch` tools for agents.
- **Skills marketplace** — search, install, and manage community skills from the web UI or CLI (`ling skills add/remove/search`).
- **`ling init` command** — scaffolds `~/.linggen/` directory tree, installs default agents, creates config, downloads skills.
- **`ling auth` command** — ChatGPT OAuth authentication (browser and device code flows).
- **Session-scoped SSE** — events are tagged with session ID; clients filter to their own session.
- **Per-session working directory** — `cd` in one session doesn't affect others.
- **SSE reconnect handling** — automatic state resync on reconnect with UI indicator.
- **Context window management** — adaptive compaction with importance-based message pruning.
- **Prompt caching** — stable system prompt prefix cached across iterations.

### Changed

- Config file renamed from `linggen.toml` to `linggen.runtime.toml`.
- Prompt system refactored from hardcoded strings to TOML templates.
- Tool calls render as individual inline widgets (aligned with Claude Code style).
- `ChatPanel.tsx` refactored into focused modules under `chat/` folder.
- `tools.rs` and `app.rs` split into module directories for maintainability.
- Default `supports_tools` changed to `true` even for unrecognized model IDs (prevents fallback to text-based JSON mode).

### Fixed

- SSE session isolation — events no longer leak across sessions.
- Streamed text-only responses no longer disappear after generation.
- Ollama 500 error — use role `"tool"` for tool result messages in native mode.
- Agent context loss on long conversations.
- Glob pattern matching edge cases.
- Queued message display order (now chronological).
- Think tag stripping for models that emit `<think>` blocks.

## v0.1.1 (2025-12-15)

Initial patch release.

## v0.1.0 (2025-12-14)

Initial release — multi-agent engine, web UI, skills system, Ollama and OpenAI providers.
