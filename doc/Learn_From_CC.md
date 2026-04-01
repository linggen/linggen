# Learn From Claude Code (Source Leak, March 31 2026)

On March 31 2026, Anthropic accidentally shipped a 59.8 MB source map in Claude Code v2.1.88 on npm, exposing 512K lines of TypeScript across 1,906 files. Below are the most relevant findings for Linggen, ordered by priority.

---

## 1. KAIROS — Always-On Daemon Mode

Our `/mission` is the closest equivalent but KAIROS is more sophisticated.

- Autonomous background agent that receives periodic `<tick>` prompts and decides whether to act or stay quiet
- Maintains **append-only daily log files** (observations, decisions, actions)
- **autoDream**: a forked subagent that does "memory consolidation" while the user is idle — merges observations, removes contradictions, converts vague insights into facts
- Key design rule: if any action would disrupt the user for >15 seconds, it gets deferred

**TODO:**
- [ ] Add autoDream-style memory consolidation as a post-mission sweep
- [ ] Consider tick-based proactive actions in mission scheduler
- [ ] Add disruption budget (defer actions that block user >N seconds)

## 2. Context Compression — 5 Compaction Strategies

Their context management is more granular than ours.

1. **Time-based clearing** of old tool results
2. **Conversation summarization** (compress old turns)
3. **Session memory extraction** (pull facts out before discarding)
4. **Full history summarization** (emergency: summarize everything)
5. **Oldest-message truncation** (last resort)

**TODO:**
- [ ] Review our compaction logic against these 5 strategies
- [ ] Add session memory extraction (save insights before compacting)
- [ ] Add time-based tool result expiry

## 3. BatchTool — Parallel Tool Execution

Claude Code has a `BatchTool` that runs multiple independent tool calls in a single turn, reducing round-trips.

**TODO:**
- [ ] Design a BatchTool / parallel tool execution mechanism
- [ ] Read-only operations should run concurrently; mutations serially

## 4. Multi-Agent Permission Mailbox

Their multi-agent orchestration has a mature permission model.

- **Coordinator Mode**: main agent assigns tasks to workers, workers execute in parallel
- **Permission Queue (Mailbox)**: workers request permission from leader for dangerous operations
- **Atomic Claim Mechanism**: prevents multiple workers from handling the same permission request

**TODO:**
- [ ] Add permission escalation pattern for delegated agents
- [ ] Implement mailbox for sub-agent permission requests
- [ ] Add atomic claim to prevent duplicate permission handling

## 5. Bash Validation (2,500+ lines)

Their bash safety validation is far more thorough than our `check.rs`.

- 2,500+ lines of bash command validation logic
- Tiered permission system per tool
- Detailed allowlist/blocklist patterns

**TODO:**
- [ ] Audit and expand `check.rs` bash validation
- [ ] Add more granular command classification (read-only vs destructive)

## 6. Anti-Distillation — Fake Tool Injection

Flag `ANTI_DISTILLATION_CC` injects decoy tool definitions into the system prompt to poison training data if someone records API traffic to train a competing model.

**TODO:**
- [ ] Low priority. Consider if we ever expose a public API

## 7. Three-Layer Memory Architecture

Very similar to what we already have. Their additions:

- **Strict Write Discipline**: index updated only after successful file write
- MEMORY.md always loaded, ~150 chars per line, stores pointers not data
- Topic files fetched on demand

**TODO:**
- [ ] Already aligned. Monitor for further details from community analysis

---

## Reference

| CC Feature | Linggen Equivalent | Gap |
|---|---|---|
| KAIROS daemon | `/mission` | autoDream, tick proactivity, disruption budget |
| 5 compaction strategies | Basic compaction | Missing extraction + time-based expiry |
| BatchTool | None | Need parallel tool execution |
| Permission mailbox | Basic delegation | No escalation pattern |
| Bash validation (2,500 lines) | `check.rs` (basic) | Much less thorough |
| Anti-distillation | N/A | Not needed yet |
| Memory layers | MEMORY.md + topic files | Already aligned |

## Sources

- [VentureBeat](https://venturebeat.com/technology/claude-codes-source-code-appears-to-have-leaked-heres-what-we-know)
- [Layer5 — 512,000 Lines](https://layer5.io/blog/engineering/the-claude-code-source-leak-512000-lines-a-missing-npmignore-and-the-fastest-growing-repo-in-github-history/)
- [Alex Kim — Fake tools, frustration regexes, undercover mode](https://alex000kim.com/posts/2026-03-31-claude-code-source-leak/)
- [WaveSpeedAI — BUDDY, KAIROS & Hidden Features](https://wavespeed.ai/blog/posts/claude-code-leaked-source-hidden-features/)
- [Kingy AI — KAIROS Deep Dive](https://kingy.ai/ai/kairos-everything-we-know-about-anthropics-secret-always-on-ai-daemon/)
