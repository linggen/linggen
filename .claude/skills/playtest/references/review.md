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

The verdict is `fail` when any hard finding stands or any answer above names a real problem; `pass` otherwise.
