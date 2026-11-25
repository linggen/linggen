# Shell memory: results and decision

## Measured: 2026-11-24 comparison

A blind comparison used the same unchanged store of about 2,000 rows and 40 real prompts, with only notes predating each prompt. A picker was shown a bounded index and could search notes; a separate judge compared its picks with the top eight vector results. The index plus agent picking won **33 prompts to 2**, with five ties, and produced less irrelevant material (41 versus 183 irrelevant notes counted).

This measured **selection after an index was shown**, not whether an agent would think to look for a memory. Picker and judge used the same model family; the index-reading and search cost was not charged against per-turn vector recall. Index limits hid some useful older notes, and keyword search missed some.

## Observed in use: 2026-11-25

Without per-turn recall, a coding agent missed two standing decisions already in notes: to reply in Chinese and a pricing decision. It did not search for them. Notes that fall out of the index remain on disk but are effectively unused if the agent does not think to search. Keyword search also struggled across languages; semantic search found relevant material that keywords missed. These are observations, not a measured failure rate.

At the time, Claude Code exposed only about the first 200 index lines and relied on the model to seek more. Codex used a summary of about 2,500 tokens and expired memories after 30 days without use. Neither solved the missing-recall problem; a read-and-forward shell can connect their memories but cannot improve what either agent remembers or decides to retrieve. Vendor behaviour may change.

## Decision

Hanli's decisive point: **Linggen memory should not forget what matters**; that is a distinction the other agents' memory systems did not provide. Main therefore keeps Linggen memory (formerly V1): one store, nightly tidy and semantic RAG recall each turn. The heavier maintenance and recall cost is accepted for now; the work is to make it leaner, not remove recall. Revisit Shell memory only if Claude Code and Codex one day share a better memory.

Ideas worth carrying into Linggen memory, subject to their own implementation and review: read other agents' memories for recall without writing them; use a status file and done entries; retain tidy rules; and attach scope and evidence to preferences so a one-off request is not mistaken for a general rule.
