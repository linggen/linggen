import { EFFORT_WORD } from '../../lib/effortChip.mts';
import { translate } from '../../lib/i18n';
import { W } from '../../lib/modelsWords.mts';

export function EffortSlider({ id, name, values, choice, disabled, onPick }: {
  id: string; name: string; values: readonly string[]; choice?: string | null;
  disabled?: boolean; onPick: (value: string) => void;
}) {
  if (!values.length) return null;
  // Always on one of the model's own levels; the engine sends the model's default when none is chosen.
  const stops = values;
  const at = Math.max(stops.indexOf(choice ?? ''), 0);
  const word = (stop: string) => EFFORT_WORD[stop] ?? stop;
  const t = (text: string) => translate(text) ?? text;
  // One row, the height of the model select beside it: 思考 · slider · value.
  // The model's name is the select's job; here it is only the tooltip.
  return <div data-testid={id} data-choice={stops[at]} title={name} className="flex h-8 w-56 max-w-full min-w-0 shrink-0 items-center gap-2 rounded-lg border border-slate-200 bg-slate-50 px-2 text-[11px] text-slate-600 dark:border-white/10 dark:bg-white/5 dark:text-slate-300" onClick={(event) => event.stopPropagation()}>
    <span className="shrink-0">{W.thinking}</span>
    <input type="range" aria-label={t(W.effort)} aria-valuetext={t(word(stops[at]))} className="h-1 min-w-0 flex-1 accent-blue-600" min={0} max={stops.length - 1} step={1} value={at} disabled={disabled} onChange={(event) => onPick(stops[Number(event.currentTarget.value)])} />
    <strong className="w-12 shrink-0 truncate text-right">{word(stops[at])}</strong>
  </div>;
}
