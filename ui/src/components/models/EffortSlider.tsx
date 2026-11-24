import { RotateCcw } from 'lucide-react';
import { EFFORT_WORD } from '../../lib/effortChip.mts';
import { translate } from '../../lib/i18n';
import { W } from '../../lib/modelsWords.mts';

export function EffortSlider({ id, name, values, choice, disabled, onPick }: {
  id: string; name: string; values: readonly string[]; choice?: string | null;
  disabled?: boolean; onPick: (value: string) => void;
}) {
  if (!values.length) return null;
  const index = values.indexOf(choice ?? '');
  const label = index < 0 ? W.automatic : (EFFORT_WORD[values[index]] ?? values[index]);
  const t = (text: string) => translate(text) ?? text;
  // One row, the height of the model select beside it: 思考 · slider · value · reset.
  // The model's name is the select's job; here it is only the tooltip.
  return <div data-testid={id} data-choice={index < 0 ? 'auto' : values[index]} title={name} className="flex h-8 w-56 max-w-full min-w-0 shrink-0 items-center gap-2 rounded-lg border border-slate-200 bg-slate-50 px-2 text-[11px] text-slate-600 dark:border-white/10 dark:bg-white/5 dark:text-slate-300" onClick={(event) => event.stopPropagation()}>
    <span className="shrink-0">{W.thinking}</span>
    {values.length === 1 ? <button type="button" className="min-w-0 flex-1 rounded border border-slate-200 dark:border-white/10" aria-pressed={index === 0} disabled={disabled} onClick={() => onPick(values[0])}>{EFFORT_WORD[values[0]] ?? values[0]}</button>
      : <input type="range" aria-label={t(W.effort)} aria-valuetext={t(label)} className={`h-1 min-w-0 flex-1 accent-blue-600 ${index < 0 ? 'opacity-50' : ''}`} min={0} max={values.length - 1} step={1} value={Math.max(index, 0)} disabled={disabled} onChange={(event) => onPick(values[Number(event.currentTarget.value)])} onPointerUp={(event) => { if (index < 0 && event.currentTarget.value === '0') onPick(values[0]); }} onKeyDown={(event) => { if (index < 0 && event.key === 'Home') onPick(values[0]); }} />}
    <strong className="w-12 shrink-0 truncate text-right">{label}</strong>
    <button type="button" aria-label={t(W.resetEffort)} title={t(W.resetEffort)} className="shrink-0 rounded p-0.5 hover:bg-slate-200 disabled:opacity-40 dark:hover:bg-white/10" disabled={disabled || choice === 'auto'} onClick={() => onPick('auto')}><RotateCcw size={11} /></button>
  </div>;
}
