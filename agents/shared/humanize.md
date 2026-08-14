# Humanize — catalog of AI-writing tells, with fixes

Shared reference for any skill that writes outward-facing text (posts,
articles, replies, copy). Not a skill itself. Load it at drafting time;
apply as a final pass over anything a reader outside Linggen will see.

Short conversational lanes (a reply, a chat message) have stricter,
register-specific rules in their own skill files; this catalog is the
general layer beneath them. Patterns follow Wikipedia's "Signs of AI
writing" plus our own flagged-text post-mortems; examples are ours.

## The stance (fix this first — it's the actual tell)

AI text reads AI because of its stance, not its words: it covers every
branch, hedges every claim, and owes the reader a complete service.
People write from one point of view, under time pressure, about the one
thing they care about.

- Say the one thing that matters. Don't answer parts nobody asked about.
- Commit. One hedge maximum, then state it plainly.
- Take a position. Neutral pro/con reporting with no opinion is a tell.
- Be specific or be silent: a named tool, a number, a moment it broke.
- Stop early. No summary of what you just said, no upbeat outlook line.

**Before:**
> There are several factors to consider when choosing a sync strategy,
> each with its own trade-offs. Ultimately, the best choice depends on
> your specific use case.

**After:**
> We went with last-write-wins everywhere except media, where delete
> wins. One conflict rule per data type, decided once.

## Content patterns

### 1. Significance inflation

**Watch for:** plays a vital/crucial/pivotal role, is a testament to,
marks a shift, reflects broader trends, setting the stage for

**Before:**
> The 1.7 release marks a pivotal moment in the project's evolution,
> reflecting a broader shift toward local-first architecture.

**After:**
> 1.7 moves sync fully off the cloud relay. Both devices talk directly
> now; the relay only carries the handshake.

### 2. Promotional language

**Watch for:** seamless, robust, powerful, stunning, cutting-edge,
boasts, rich, comprehensive

**Before:**
> The app boasts a seamless pairing experience and a comprehensive suite
> of powerful media tools.

**After:**
> Pairing is one QR scan. After that your music and photos sync on their
> own.

### 3. -ing padding

**Watch for:** trailing "…, highlighting/showcasing/ensuring/reflecting…"
clauses that add fake depth

**Before:**
> The daemon restarts automatically, ensuring reliability and
> highlighting the system's self-healing design.

**After:**
> The daemon restarts itself. If it's down more than a second or two,
> something else is wrong.

### 4. Vague authority

**Watch for:** experts believe, industry reports, many users say, studies
show — with no source

**Before:**
> Many developers agree that local-first architectures are the future of
> personal software.

**After:**
> Our sync bug reports dropped to zero after the move — there's no server
> state left to disagree with the device.

## Language patterns

### 5. AI vocabulary

**Watch for:** delve, tapestry, landscape (abstract), interplay,
underscore, foster, leverage, utilize, facilitate, streamline,
multifaceted, myriad, embark → use the plain word (use, help, many, start)

**Before:**
> We leverage embeddings to facilitate seamless retrieval across the
> memory landscape.

**After:**
> Memories are embedded once and searched by meaning, so "the stripe bug"
> finds the row even if you never wrote the word stripe.

### 6. Copula avoidance

**Watch for:** serves as, stands as, represents, functions as, features,
offers → "is", "has"

**Before:**
> The engine serves as the core runtime and features a skill system that
> functions as its extension mechanism.

**After:**
> The engine is the runtime. Skills extend it.

### 7. Negative parallelism

**Watch for:** not just X, but Y · it's not about X, it's about Y

**Before:**
> It's not just a music player — it's a whole way of owning your library.

**After:**
> Your library lives in a folder on your Mac. The phone mirrors it,
> offline included.

### 8. Rule of three

**Watch for:** exactly three adjectives, examples, or clauses, repeatedly

**Before:**
> Fast, private, and reliable — sync that just works across your phone,
> your Mac, and your life.

**After:**
> Sync finishes in the background before you'd think to check on it.

### 9. Synonym cycling

**Watch for:** the app → the tool → the platform → the solution

**Before:**
> The app syncs your music. The tool also handles photos. The platform
> keeps both offline.

**After:**
> The app syncs music and photos, and keeps both offline.

### 10. False ranges

**Watch for:** "from X to Y" where X and Y aren't on a scale

**Before:**
> From karaoke lyrics to budget tracking, from photo cleanup to CarPlay,
> the app covers everything.

**After:**
> Four things live in it: music, budgets, photo cleanup, and the Mac
> agent.

## Style patterns

### 11. Em-dash chains

**Before:**
> The scan runs locally — no cloud involved — and finishes fast — usually
> under a minute.

**After:**
> The scan runs locally, no cloud involved. It usually finishes in under
> a minute.

### 12. Format tells

**Watch for:** bold-label bullets ("**Speed:** it's fast"), Title Case In
Headings, emoji decoration (🚀✅💡), "Key takeaways" blocks, wrap-up
conclusions

**Before:**
> - **Privacy:** Your data stays local.
> - **Speed:** Sync is instant.
> - **Reliability:** It just works.

**After:**
> Your data stays on your devices, and sync is fast enough that you
> won't catch it running.

## Communication patterns

### 13. Assistant artifacts

**Watch for:** Great question, I hope this helps, Certainly!, Let me know
if…, Would you like me to…, You're absolutely right, as of my last update

**Before:**
> Great question! Here's an overview of how pairing works. I hope this
> helps — let me know if you'd like more detail!

**After:**
> Pairing is a QR scan on the LAN, or a code if you're remote. The
> window only exists while the QR is on screen.

### 14. Excessive hedging

**Watch for:** could potentially, might arguably, it could be argued,
somewhat, to some extent

**Before:**
> This approach could potentially reduce sync conflicts to some extent
> in certain scenarios.

**After:**
> This kills the double-delete conflict. The clock-skew one is still
> open.

### 15. Generic upbeat conclusions

**Watch for:** the future looks bright, exciting times ahead, a major
step forward, journey toward excellence

**Before:**
> This release represents a major step forward, and we're excited about
> the journey ahead.

**After:**
> Next up: the karaoke view drops its last HTTP call.

### 16. Uniform rhythm

Every sentence medium-length and balanced is itself a tell — vary hard.

**Before:**
> The update improves the pairing flow. The update also fixes the sync
> bug. The update includes minor performance work.

**After:**
> Pairing got a rework. The sync bug from 1.6 is gone. Assorted small
> speedups too.

## Document scale

### 17. Full-piece transformation

Sentence fixes aren't enough if the whole piece has the AI shape:
grand opening, three balanced body paragraphs, upbeat close. A human
piece is built from one real observation and its evidence.

**Before:**
> Local-first software marks a pivotal shift in today's rapidly evolving
> technological landscape. By keeping data on-device, these groundbreaking
> tools enhance privacy, foster user trust, and streamline the experience.
>
> At its core, the value proposition is clear: it's not just about
> storage — it's about ownership. Industry observers note that adoption
> is accelerating across the ecosystem.
>
> In conclusion, the future of local-first computing looks bright, and
> exciting times lie ahead.

**After:**
> I moved my music library off streaming and into a folder my phone
> syncs from my Mac. The surprise wasn't privacy — it's that everything
> got faster. Search is instant because it's a local index. Nothing
> buffers.
>
> The catch is sync conflicts. Two devices editing one playlist forced
> us to actually pick a rule (last write wins, except deletes on media).
> Streaming services solve this by just owning your data.
>
> Worth it? For music, clearly. For photos I'm still not sure — my
> library is 200 GB and the first sync took a night.

### 18. Soulless vs alive

Removing every tell can still leave dead text — technically clean,
nobody home. The fix isn't more cleanup; it's an opinion, a reaction,
a first person with something at stake.

**Before (clean but dead):**
> The test suite was migrated to the new runner. Execution time
> decreased from 40 to 12 minutes. Some flaky tests were identified
> during the migration. The team will address them in a future sprint.

**After (someone's home):**
> Migrating the suite to the new runner cut CI from 40 minutes to 12.
> The migration also outed six tests that only ever passed because the
> old runner ran them in a lucky order — honestly the more valuable
> result. Those are quarantined for now, which is a nicer word for
> "we'll deal with it later."

## Final checklist

1. Would a busy person with a stake write this sentence? If it could
   only come from a neutral advisor with unlimited time, rewrite it.
2. Scan for the watch-for lists above; fix every hit.
3. Read it aloud. Uniform rhythm means it needs one short sentence and
   one cut.
4. Delete the last sentence if it summarizes, moralizes, or cheers.
