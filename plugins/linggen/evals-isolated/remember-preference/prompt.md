---
description: A standing rule the user states lands as a search-first, indexed, user-voice semantic preference.
tags: [capture]
max_turns: 12
allowed_tools: [Skill]
append_system_prompt: |
  # MCP Server Instructions

  ## plugin:linggen:linggen
  Linggen memory works without an account. Save durable facts about the person, not project internals or secrets.
  Core is tiny and always loaded: stable identity (name, role, location, languages, family). Semantic holds only what lasts: preferences, standing rules, decisions. Time-bound (go-ahead, progress, state, price) gets NO tier (episodic). replace_ids keeps the losers' top tier: to move down pass tier=episodic.
  Before saving core or semantic, search the subject. Mark the user's words from=user. Merge your own derived rows atomically with memory_add + replace_ids; never add then delete. Before replacing or rewriting a user row, show its full text and date and ask; set user_directed:true only after their answer or an explicit settled instruction. Keep tier and scope unless directed; supersede old status in the same write. Cite recalled facts you use.
  For each background agent_run, including a follow-up, start a new background watcher at the returned /api/agent/wait WebSocket URL. Answer permission_request with agent_permit: allow_once, allow_session (path/folder for this session), or deny. Outward requests take allow_once or deny only; the deny floor cannot be granted. No reply in 10 minutes means deny. Read reply, followup, error and cannot_run from finished. If the watch ends first, call agent_status (op=session) once; if still running, start a new watch. Do not poll. Check pending and denied. Continue with session_id without widening grants.
  On a usage limit, read its reset time (cannot_run.resets_at), then pick a ready model from agent_status (op=models) or come back after it; never retry before.
  Browser tools control one visible Chrome tab; actions can prompt for per-site permission in Chrome. Read again after page changes. For memory upkeep use the dream, scan or solve runbook; memory_upkeep (op=dream_status) shows backlog and failures.
  The app tool uses what the person's apps do on this Mac, such as playing music: app op=list, then op=call. A private app stays closed until the owner opens it in Settings.
---

From now on, always write my commit messages in English, even when we chat in Chinese. Please remember that.
