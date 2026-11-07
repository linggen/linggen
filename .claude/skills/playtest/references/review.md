# Answering the checklist — what each question is after

The four questions are asked of every screenshot (`checklist.md`). Answer from what the picture shows, in one or two
lines; name the thing ("the 3rd card's cost", "the band under 精精's rank"), never "looks fine".

1. **Can every card's cost, name and effect be read?** Each card in the hand and on the ranks: the cost gem whole and
   not under a neighbour, the name not cut, the effect line legible at that size. A raised (held) card may cover its
   neighbour's top edge — never a cost. On the phone sizes a hand of six or more lies in two rows.
2. **Is it clear what was won?** After a win: a 所得 card with what came (修为 / 灵石 numbers, a new card's face, the
   things into the 储物袋). A title with nothing under it is a fail. After a loss or a withdrawal: the page says which.
3. **Anything confusing or wasted?** An empty band taller than a card between blocks, a sheet whose purpose is unclear,
   a refusal line that does not say why (「斗法未完」 alone was the 11-04 bug), two controls that look the same and do
   different things.
4. **Does anything look broken?** Overlap, text cut mid-word, a control half off the screen, a blank or half-drawn
   panel, an English string in a zh run, an error toast.

Hard findings in `findings.json` already have their screenshot; say whether the picture agrees with the check (a lint
can be wrong — if it is, write so in the note, and the lint gets fixed, not ignored).

## Sure or unsure

Every shot is sorted three ways. **Sure pass**: the four answers are clean. **Sure fail**: an answer names a real
problem — mark the note `fail: true`; it fails the review at once. **Unsure**: the answer depends on taste or on how it
reads on a real phone — the only two reasons. A defect you can name is never unsure; a doubt you could settle by
looking harder is not either (look harder).

Before marking one unsure, look in `tools/playbot/rulings.jsonl` for the same question (the same flag kind/why, a phone
size or the Mac, the same kind of thing asked): if Hanli has ruled, follow him and write the note sure.

An `ask` is one question he can answer 行 / 不行 from the picture alone, in plain Chinese, ≤ 40 字, naming the thing:
- good: 「空的两排之间这条空带，算浪费吗？」 · 「杀招折成一行，手机上看得清吗？」
- bad: 「这张有问题吗？」 (names nothing) · 「layout OK?」 (English, no thing) · two questions in one.

A `box` `{x, y, w, h}` in the shot's own pixels (the shot's size is its viewport: 430×932, 375×812, 1280×820) frames
the place to look — the band, the line, the button. Leave it out only when the whole shot is the question.

The verdict is `fail` when any hard finding stands or any sure answer above names a real problem; `pending` while an
unsure one waits for Hanli; `pass` otherwise.
