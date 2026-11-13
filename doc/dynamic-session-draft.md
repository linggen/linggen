# Dynamic session — discussion draft (2026-11-13)

**Status: talk only, nothing decided, nothing built.** Hanli and Claude,
2026-11-13. To be reviewed in detail the week of 2026-11-16, once there is token
quota. The perception half of this talk *was* agreed and is already in
`perception-spec.md` (commit `b2808772`); this file holds the session half.

---

## 1. How it started

Two screenshots: the Mac's `Yinyue · 2026-11-13` session (all 银月 lines) and a
DJ `skill` session (one Ling line). Hanli: "are Ling and Yinyue sharing one
session now?"

What was true that day:

- **Phone:** one Chat thread (`yinyue/thread.json`, resumed within 24 h, else
  fresh). Ling and Yinyue both speak in it; app tools come in packs per turn.
  灵境 boards that need a model (讨饼债, 聊天, 画猜) run their own one-off
  context, outside the Chat. The rules run on the phone with no model.
- **Mac:** one session per app (DJ, 灵境, …, resumed within 24 h), plus
  Yinyue's own daily session, which nobody types in — event turns only.
- No session sync between devices (`one-ling-spec.md`).

## 2. Hanli's words, in order

1. 「apps的session合成一个, 比较简洁, yinyue和ling可以了解所有apps的情况,
   符合设计定位, ai agent 知道所有事情. 但是会浪费token. 分开, 合并各有好处.
   我们手机合并, mac分开, 感觉不太一致.」
2. Pushback on Claude's "phone turns are short": 「手机用户可以独立玩lingjing,
   不能说手机的turn比mac短吧?」 — he was right; that argument was dropped.
3. 「我们有one ling 设计, yinyue和ling都是全局only one的, 他们是linggen的魂,
   是用户的personal AI agents. 设计上我们想告诉用户, linggen里有你的两个助理,
   知道你的全部事情, 与你一起工作, 娱乐, 生活. 手机上的感觉更对一些, 只有一个chat,
   里面有ling和yinyue. mac上似乎分开了. 但问题也不大. 只是session比较多,
   yinyue的日常session感觉和手机chat比较像 … 而且现在lingjing的设计也是关联到了
   其他app, 有日常任务奖励分享健康等其他app活动. session结构上怎么能调整一下,
   让一个session能把世界(所有app里发生的事儿)放进去, 又不至于把工具, 系统prompt
   撑爆 … 是否可以让chat(session)知道全linggen世界, 但目前用户在玩lingjing,
   focus在lingjing上.」
4. On knowing the past: 「我们想让agents知道所有发生的事儿, 这个方向是对的吗?
   还是agent只知道当前发生的事儿就够了. 过去发生的, 应该压缩, 保存成记忆就够了? …
   我感觉能有工具让agent查看过去发生了什么就OK了, 我们可以有个log记录world state.
   用户的健康变化, 完成了一个workout, 下载了几首粤语歌, 用户每天的行为都记录下来,
   让agent在需要的时候查看. 这些log不一定要写进记忆??」
5. 「90days也不够, 最好能有个数据库可以查询. 总结. 但感觉有朝着memory方向发展了.
   lanceDB加一个event table? 还是文件就可以?」 → asked how fast file search is
   → measured → 「ok, 按你说的做, 更新一下设计文档吧」 (done, `b2808772`).
6. Mac sessions: 「一个session可以有world状态, apps的session合并, 和手机一致?
   这个要合并吗?」 → 「总体来说没问题.」
7. **The dynamic-session idea (his):** 「动态session, 根据需要load context.
   传统agent session会把context发给llm api, 好处是可以利用cache省token, 但compact后,
   compact之前的context就不再发了, 对吧? 我们是否可以更进一步, 把session context
   更加动态, 比如focus在lingjing, 就发session中lingjing相关的context, 或者简单点,
   切换apps时compact一次. 但可能cfo里只说一句话, 就没必要compact了, 可以compact
   加条件, 比如30% context额度时自动压缩. 动态session|context的好处是session数量少,
   有今天全部apps里的上下文. 用户切换app时不必切换session, 给用户一个ling|yinyue
   无处不在的感觉, 一直是同一个ling和yinyue在伴随用户. apps可以切换, 但用户是同一个,
   agent也是同一个.」
8. His own weighing: 「好处就是把每天多个apps的多个session合并成了一个,
   本地持久化的session文件少一些. 坏处是提高了处理的复杂度, 有可能浪费token.」

## 3. Claude's analysis so far

### 3.1 "Knows everything" comes from the world, not the transcript

An app's conversation sitting in the same thread is the weakest way to know
about it: it vanishes at the next 24 h reset or compaction. What makes Ling and
Yinyue know all of Linggen is perception (agreed, `perception-spec.md`):

- **Now** — one line per app in every turn's "Right now", declared by the skill
  (~300–500 tokens for all apps).
- **What happened** — the activity log, kept, rolled up monthly, read through
  `recent_activity` with filters, mirrored between phone and Mac by day.
- **What it means** — memory; the dream reads the log as evidence and writes
  patterns only.

This works with today's sessions too. The session question is second-order.

### 3.2 Context per turn, in layers (only one is large)

| Layer | Holds | Size |
|---|---|---|
| Soul | Ling's / Yinyue's persona | fixed, cached |
| World | one line per app + doorbell | ~300–500 tokens |
| Focus | the focused app's skill prompt + all its tools | the big one, one app |
| Reach | 1–3 common verbs per other app + `open_toolbox` | ~1–2k |

Focus is decided by fixed signals (the page the user is on, the app screen just
left on the phone), never by the model. The phone already has packs and
`open_toolbox`; the Mac would need them ported.

### 3.3 The dynamic session: keep the thread, project the prompt

Agreed with the idea; the refinement is **never compact the stored thread —
rebuild the prompt from it every turn**:

```
[Soul + Right now]                                   stable → cache hits
[non-focus app segments: small verbatim, big summarised]  changes on focus switch only
[last few turns, any app]                            context right after a switch
[focus app segment, verbatim + its tools]
[this turn]
```

- Each turn is tagged with its focus app when sent. A "segment" = the day's
  turns tagged with one app.
- **Fold only when needed** (his conditional compaction): a non-focus segment
  becomes a summary only when it is big (a few thousand tokens) or the whole
  projection passes a budget (his 30%). A one-line CFO question stays verbatim.
- A summary is computed once when its segment leaves focus, stored, reused;
  recomputed only after that app is focused and changes again.
- The focus segment, if it grows too long (a long 灵境 game), compacts within
  itself like ordinary compaction.
- Returning to an app brings its full segment back — the thread kept it.
- **Cache:** providers cache by prefix, so stable parts first and summaries in a
  fixed order. A focus switch costs one cache write; every later turn in that
  focus hits.
- **This removes the 灵境 exception** Claude had proposed earlier (灵境 play as
  its own session): its long play is folded when it is not in focus.

Risks and their answers:

1. Cross-app references ("the song mentioned in 灵境") — the last few turns are
   always verbatim; older text comes back through a `recall_thread(app, query)`
   tool, the same "door when needed" pattern as `recent_activity`.
2. Focus misjudged — fixed signals, turn tagged at send time.
3. Ling and Yinyue each get their own projection — already the per-member
   context rule in `shared-session-spec.md`.

What the user would see on the Mac: one Chat (plus missions and coding
sessions); the DJ / CFO / 灵境 pages are focused views of it; Yinyue's daily
session folds into it. Phone and Mac still keep their own thread each.

### 3.4 Hanli's cost/benefit, and Claude's take on it

- **Benefit:** fewer sessions and files; the day's context across every app;
  "the same Ling and Yinyue everywhere".
- **Cost:** more processing complexity (tagging, projection, folding, summary
  cache, `recall_thread`); possible token waste.
- Claude's view: token cost is likely **lower**, not higher, than either
  extreme — one switch costs one cache write, and the world layer replaces
  carrying other apps' history. The real cost is complexity, and it is real:
  projection + folding is a new piece of the engine that every turn depends on.
  Worth measuring before building (see §5).

## 4. Order proposed (each step useful on its own)

1. **World layer** — skill-declared world lines + apps writing their verbs to
   the activity log (perception-spec, agreed). Makes "they know everything"
   true with today's sessions.
2. **Focus + reach on the Mac** — port the phone's packs.
3. **Dynamic session** — one thread, projection, folding. Last, biggest.

## 5. Open, for the review

- **How long a thread lives.** Claude recommends one thread per day (like
  Yinyue's daily session and the phone's 24 h rule; tomorrow starts with a short
  summary of today, older reaches via memory and the log). Alternative: a
  thread that never ends, relying on folding — more continuous, much more
  complex.
- **Is the complexity worth it**, versus steps 1–2 plus today's per-app
  sessions? A measurement would settle it: replay one real day's Mac sessions
  through a projection and compare tokens sent and cache hits.
- **Fold thresholds:** segment size and the whole-projection budget (30%?).
- **Summary model:** which model writes the fold summaries, and their size.
- **Mac pages as views:** does the DJ page show only DJ turns, or the whole
  Chat scrolled to DJ?
- If adopted: rewrite `shared-session-spec.md` (projection, folding, cache
  order, `recall_thread`), `one-ling-spec.md`, `session-spec.md`.
