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
  return <div data-testid={id} data-choice={index < 0 ? 'auto' : values[index]} className="w-40 max-w-full min-w-0 shrink-0 rounded-lg border border-slate-200 bg-slate-50 px-2 py-1 text-[11px] text-slate-600 dark:border-white/10 dark:bg-white/5 dark:text-slate-300" onClick={(event) => event.stopPropagation()}>
    <div className="flex items-center gap-1.5">
      <span>{W.thinking}</span><strong>{label}</strong>
      <span className="min-w-0 flex-1 truncate" title={name}>{name}</span>
      <button type="button" aria-label={t(W.resetEffort)} title={t(W.resetEffort)} className="rounded p-0.5 hover:bg-slate-200 disabled:opacity-40 dark:hover:bg-white/10" disabled={disabled || choice === 'auto'} onClick={() => onPick('auto')}><RotateCcw size={11} /></button>
    </div>
    {values.length === 1 ? <button type="button" className="w-full rounded border border-slate-200 dark:border-white/10" aria-pressed={index === 0} disabled={disabled} onClick={() => onPick(values[0])}>{EFFORT_WORD[values[0]] ?? values[0]}</button>
      : <input type="range" aria-label={t(W.effort)} aria-valuetext={t(label)} className={`block w-full accent-blue-600 ${index < 0 ? 'opacity-50' : ''}`} min={0} max={values.length - 1} step={1} value={Math.max(index, 0)} disabled={disabled} onChange={(event) => onPick(values[Number(event.currentTarget.value)])} onPointerUp={(event) => { if (index < 0 && event.currentTarget.value === '0') onPick(values[0]); }} onKeyDown={(event) => { if (index < 0 && event.key === 'Home') onPick(values[0]); }} />}
  </div>;
}
