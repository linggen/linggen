# Write-side memory eval

Scores the memory pipeline's **write side** — what gets stored, not
what gets retrieved: extraction (save the durable, skip the noise),
routing (events vs state, tier placement), dedup, reconciliation
(contradictions ask, never silently merge), decay (dream promotes,
sweep evicts), secrets. The complement of the LongMemEval retrieval
check, which rewards hoarding and must never be optimized toward.

Every scenario runs through the **real engine over HTTP** — production
prompt assembly, per-turn recall, capture protocol, real dream
mission — against a **throwaway store** (own `LINGGEN_DATA_DIR` +
ling-mem port, deleted afterward). The user's real store is never
touched.

## Scenario format (`scenarios/*.yaml`, one list per axis)

```yaml
- name: kebab-name
  axis: extraction | routing | dedup | reconcile | decay | secrets
  description: one line
  fixture:                  # optional rows imported before the turns
    - content: "..."
      type: preference      # fact|preference|decision|tried|fixed|learned|built
      from: user            # user|agent|derived
      tier: semantic        # core|semantic|episodic
      age_days: 30          # runner backdates created_at/occurred_at
  turns:                    # user messages, sent sequentially, each waits for the reply
    - "..."
  ask_user:                 # optional; default expect: false
    expect: true
    answer: "..."           # sent via /api/ask-user-response when the widget fires
  dream: true               # run the dream mission (remember + sweep) after the turns
  expect:
    must:                   # LLM-judged predicates over the end-state store
      - predicate: "..."
        tier: semantic      # optional tier constraint
    must_not:
      - predicate: "..."    # judged
      - contains: "TOKEN"   # mechanical substring across all rows
    subject_max_one:        # dedup: at most one row may assert this subject
      - "query phrase"
```

## Scoring

- Mechanical first: `contains` traps, tier placement, subject counts,
  post-sweep survival, AskUser fired/not (from the run transcript).
- Judged: each `must` / judged `must_not` predicate is a binary
  question to the judge model with the candidate rows attached; every
  verdict cites a row id, so failures are debuggable and verdicts are
  spot-checkable.
- Scorecard: per-axis pass rates + extraction precision/recall.
  Model-in-loop is nondeterministic — compare runs as means, not
  single passes.

Gold labels are small and human-reviewable by design: a disagreement
with a label is a spec bug worth having found.
