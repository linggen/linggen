---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Permission System

Permissions control what an agent may do and where. They belong to a session, not to a device, account token, or directory shared across sessions.

## Owner and room users

An authenticated owner is the owner whether accessing their Linggen locally or remotely, including through a browser. Owner sessions use the permissions below.

A room user running Linggen on their own device uses their own device's permissions; sharing a model or token does not give them control of the owner's device. Room browser consumers currently have capabilities limited by the owner's room configuration and cannot approve additional permissions. The intended simpler browser-chatter model is **chat only**, with no permission prompts; it is not yet established here as implemented. See `room-spec.md` for the current room model.

[Hanli]: we need to figure out how do we define owner, when there are multiple accounts. 

## Remote trust: devices and memory

Anything reaching this machine over the network presents a paired device's token at the LAN gate. Each paired device is one row with a kind:

- **Machine** — the owner's other computer, approved with Allow at this machine. For memory it is the owner: it sees all of the owner's memory, through `/mcp/memory` and the replica protocol.
- **Phone** — paired with the on-screen code. For memory it is its own person and sees only that person's rows, through `/api/memory/<verb>`; the machine routes refuse it.

Through the WebRTC tunnel a request arrives from loopback, so the tunnel names the device it carries and marks what it knows of a peer that names none; only this machine's own page and the owner's relay login count as the owner. A request into a session (a day's lookup, a page's history, a chat, an agent's notes) is sent as the person behind the device, resolved as memory resolves it, whatever `user_id` it names: by the tunnel for a peer, by the LAN gate for a token on plain HTTP. Loopback keeps trusting its caller. Approving a machine (Allow / Deny) is only for someone at this machine. See `memory-spec.md` § Who reaches memory.

The same kinds decide two more doors. The MCP front door (`/mcp`) is for computers: its tools act as the owner, so this machine and an approved machine come in and a phone is refused (403). `/api/ling-mem/config` (memory's keep-days and recall threshold) answers the owner rule only: this machine and an approved machine; a phone and a caller nobody knows get 403. `GET /api/user/name` answers with the asker's own name: core memory's for the owner, the pairing record's for a phone that is another person, none for a signed-out phone, and 403 for a caller nobody knows.

A phone names its device as its channel opens, and its first requests can overtake that message. A request from a peer that has not named itself waits up to 1.5 s, counted from the peer's first request, for the name to arrive.


## Session grants

Each grant pairs a permission level with a scope: a directory and its children, or **all directories** (`*`). The most specific directory grant takes precedence; the all-directories grant is the fallback. Without a matching grant, the level is `chat`.

| Level | Agent capability |
|:---|:---|
| **chat** | Conversation and chat-tier capabilities; gated actions require permission. |
| **read** | Read-class tools and commands. |
| **edit** | Read plus file editing and other edit-class actions. |
| **admin** | Edit plus admin-class actions, including higher-risk commands. |

These levels apply uniformly to individual directories and all directories. All-directories admin is broad access, not macOS administrator or root privilege. The system's hardcoded deny floor still blocks certain dangerous agent commands regardless of grants.

**Grants are kept per place.** A session's turns each stand in a place — the app whose page the turn came from, or home (`session-spec.md` § A turn's place) — and each place has its own grants in the session's permission file: a grant the user gave while in one app is not honoured in another app's turn of the same session, and is still honoured when a turn is back in the first app. The session's own place (its skill, or home) keeps its grants where the file always held them, so a file written before places reads as the grants of the session's own place; another place's grants sit under that place's name. A subagent uses the grants of the place its parent's turn stands in. The permission modes and the hardcoded deny floor are the same in every place. Today every turn of a session stands in the session's own place, so a session has one set of grants in practice; the badge, the per-chat grants and `agent_permit` show and change that set.

A new user session starts with the configured default level on its starting directory, normally `read`. A place's first turn in a session starts the same way, on the place's folder. Session grants survive reopening the same session; they are not automatically inherited from another session. Missions and skills may provide their own grants. Skills may also persist and reapply user-configured grants across sessions; this is distinct from inheriting another session's grants.

## Asking for permission

When an interactive agent action needs a higher level, the chat asks the current session's user. The request names the action and shows the target path. It offers the needed level on each directory from the repository root down to the file's directory, so one click can cover the whole repository:

- **Needed level on a directory**, one button per directory. For `a/b/c/file`, the buttons are `a`, `a/b`, and `a/b/c`.
- **Needed level everywhere**.
- **Admin everywhere**, added only when the needed level is below admin.
- **Deny**.

An admin action omits the extra admin choice. A path with no repository root stops before the home directory. `/` and system directories such as `/dev` are not offered.

Deny blocks this action and closes its project (the nearest git root, else its folder; never the home folder) to every tool for the rest of the run: a later call there is refused without asking, the chat shows one line saying so, and the agent is told to mark what needed it unverified and continue. Nothing is stored: the next run asks again. Either allow choice is written to that session's permission file. Reopening the same session keeps it. Other sessions do not inherit it. A grant for all directories replaces existing directory grants; a later, more specific directory grant can override it. Admin everywhere is the broad setting: later actions in that session are not asked again. The hardcoded deny floor still blocks dangerous commands. Unattended runs cannot ask and must pause or fail when permission is insufficient.

### Sessions that ask their caller

A session started through MCP `agent_run` has no person in the chat, but it has a caller. An action beyond its grants asks the caller instead of failing: the request (action, target path, needed level, the agent's stated reason) waits on the session's wait stream and in `agent_status`, and the run waits for `agent_permit`. The caller may allow the action once, grant its path or its folder at that level for the rest of the session, or deny; a deny closes the project for the run as above. The caller is never offered a grant for all directories. No answer within 10 minutes counts as deny. A refused action is listed as denied. The hardcoded deny floor is never asked. The owner sees the same request in the chat and may answer it there.

The chat shows the effective level for the current working directory. Changing that directory updates the displayed level without changing the grants. Changing its level affects that directory, not unrelated directories.

## Boundaries and exceptions

- Reading outside a grant also requires approval; reads are not universally free. Temporary scratch directories are exempt from path-mode checks, but not the Bash deny floor: the path is free, the command is not — one that runs code (`sh /tmp/x.sh`, `python3 -c … > /tmp/out`) with no other path to judge is judged at the session's folder, like a command with no path. A session may read its own session folder (its background tasks' logs) without a grant; writing there, or reading another session's folder, is gated as usual.
- Bash commands are classified by risk and checked against identifiable targets. This is **best effort, not a filesystem sandbox**: a command may access files, processes, or network resources beyond what can be inferred from its arguments. A directory-scoped admin grant must not be presented as shell confinement. Text a command carries is not a target: a heredoc body fed to `cat` or `tee`, and the pattern or program of `grep`, `rg`, `sed`, `awk` and `jq`. Only a shape the gate reads exactly as the shell does is dropped: a body any other program reads, one the shell expands (`$`, backticks under an unquoted delimiter), or one in a command where any line outside the bodies holds a pipe into the reader, grouping, a comment, a substitution, a backslash, a compound command (`if`, `while`, `{`), an assignment or a builtin that changes the shell (`alias`, `exec`, `eval`) is still judged, and so is every word of a pattern command whose flags it cannot read. A flag counts as taking a value only when every implementation takes one (BSD `sed -l` and `grep --context` do not; BSD `sed -i ''` is not read specially, as GNU sed takes the word after it as a file). A command with a heredoc left in, with a quote character or backslash inside a comment, or with unquoted whitespace the shell does not split on (a carriage return, a no-break space), is unclear — its words are not the shell's — and is judged as admin. A `sed` or `awk` script that names a file itself (`w file`, `r file`, `print > "file"`, `getline < "file"`) is judged as admin, like one that runs a command: that file is not a target the gate can see. Where seds and awks disagree on which word is the script (BSD `sed -l`, a bare `-i` on macOS, `awk --assign x` under the one-true-awk), the command is admin or every candidate word is read as the script. A redirect target, glued to its operator or not (`>/x`, `2>>/x`, `</x`), and a file argument are always targets; `/dev/null` and the standard streams are not files.
- Skills and some chat-tier or explicitly ungated MCP tools can operate outside the ordinary path-mode gate. A skill may request its own grants. The owner should not assume that `chat` alone is a complete isolation boundary.
- Browser actions use site trust and separate confirmations for sensitive actions; see `browser-control-spec.md`.
- Agent permissions do not gate the owner's own typed commands or deterministic skill interface actions. This is not a substitute for enforcing identity and access boundaries on remote interfaces.
- A chat has one set of grants for each place (§ Session grants). Subagents at every depth use the set of their parent turn's place live instead of keeping their own copy. A subagent's permission prompt asks the person for the chat, and a grant made there applies at once to the main chat, the badge and every subagent. A subagent can never raise a grant without the person. A delegated run with no session to share runs on a copy of its parent's grants.

## Related documents

`tool-spec.md`, `session-spec.md`, `agent-spec.md`, `mission-spec.md`, `webrtc-spec.md`, `room-spec.md`, and `browser-control-spec.md`.
