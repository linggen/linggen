import { W, say } from '../../lib/modelsWords.mts';
import { translate } from '../../lib/i18n';
import type { UseEdit, UseOption, UseRow, Uses } from '../../lib/modelUses';
import { EFFORT_WORD } from '../../lib/effortChip.mts';
import { cardCls, hintCls, inputCls } from './shell';

const optionName = (option: UseOption) => ({
  qwen: 'Qwen3-TTS', system: W.systemVoice, local: W.thisMac, 'local-stt': 'Whisper',
  ...EFFORT_WORD,
} as Record<string, string>)[option.id] ?? option.label;
const status = (option: UseOption) => ({
  ready: W.ready, no_key: W.liveMissingKey, signed_out: W.notSignedIn, unavailable: W.notReady,
  download: option.disk_gb ? say(W.downloadSize, { n: option.disk_gb }) : W.notDownloaded,
})[option.state];
const t = (text: string) => translate(text) ?? text;

function UseLine({ id, label, row, busy, onPick, automatic, downloadable = false }: {
  id: string; label: string; row: UseRow; busy: boolean;
  automatic?: string; downloadable?: boolean; onPick: (choice: string) => void;
}) {
  const canPick = (o: UseOption) => o.state === 'ready' || (downloadable && o.state === 'download');
  const choices = row.options.filter(canPick);
  const current = row.choice ?? (automatic ? '' : row.effective ?? '');
  const picker = choices.length + Number(!!automatic) >= 2;
  const facts = picker ? row.options.filter((o) => !canPick(o) || o.state !== 'ready') : row.options;
  return (
    <div className="grid gap-1.5 border-b border-slate-100 py-2.5 last:border-b-0 sm:grid-cols-[8rem_minmax(0,1fr)] dark:border-white/5">
      <label htmlFor={picker ? `default-${id}` : undefined} className="self-start pt-1 text-[13px] font-semibold">{label}</label>
      <div className="min-w-0">
        {picker && <select id={`default-${id}`} data-testid={`default-${id}`} className={`${inputCls} w-full`} value={current} disabled={busy} onChange={(e) => onPick(e.target.value)}>
          {automatic && <option value="">{automatic}</option>}
          {!automatic && !current && <option value="" disabled>{W.notSet}</option>}
          {current && !row.options.some((o) => o.id === current) && <option value={current} disabled>{current}</option>}
          {row.options.map((option) => <option key={option.id} value={option.id} disabled={!canPick(option)}>{optionName(option)}</option>)}
        </select>}
        {!!facts.length && <div data-testid="use-line" className="flex flex-wrap gap-x-4 gap-y-1 py-1 text-xs">
          {facts.map((o) => <span key={o.id}>{optionName(o)} <span className={hintCls}>· {status(o)}</span></span>)}
        </div>}
        {picker && automatic && row.effective && row.effective !== row.choice && <p className={`${hintCls} mt-1`}>{W.liveFirst}: {row.options.find((o) => o.id === row.effective)?.label}</p>}
      </div>
    </div>
  );
}

export function DefaultCard({ view, busy, failed, retry, pick }: {
  view: Uses | null; busy: boolean; failed: boolean; retry: () => void; pick: (edit: UseEdit) => void;
}) {
  const shared = { busy };
  const fallbacks = view?.chain.slice(1).map((id) => view.chat.options.find((o) => o.id === id)?.label ?? id);
  const liveCanPick = (view?.live.options.filter((o) => o.state === 'ready').length ?? 0) >= 2;
  return (
    <section className={cardCls} data-testid="default-card">
      <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300">{W.defaultModel}</h2>
      <p className={`${hintCls} mt-1`}>{W.defaultModelHint}</p>
      {failed && <p role="alert" className="mt-2 text-xs text-red-600">{W.defaultsFailed} <button type="button" className="underline" onClick={retry}>{W.retry}</button></p>}
      {!view && !failed && <p className={`${hintCls} mt-2`}>{W.loading}</p>}
      {view && <div className="mt-2">
        <UseLine {...shared} id="chat" label={W.chat} row={view.chat} onPick={(choice) => pick({ use: 'chat', choice })} />
        {!!fallbacks?.length && <p className={`${hintCls} py-1 sm:pl-32`}>{say(t(W.fallbackChain) as typeof W.fallbackChain, { models: fallbacks.join(' → ') })}</p>}
        {!!view.effort.options.length && <UseLine {...shared} id="effort" label={W.effort} row={{ ...view.effort, choice: view.effort.options.some((o) => o.id === view.effort.choice) ? view.effort.choice : null }} automatic={W.automatic} onPick={(choice) => pick({ use: 'effort', choice: choice || 'auto' })} />}
        <UseLine {...shared} id="live" label={W.liveVoice} row={view.live} automatic={liveCanPick ? W.automatic : undefined} onPick={(choice) => pick({ use: 'live', choice: choice || null })} />
        <UseLine {...shared} id="speak" label={W.readAloud} row={view.speak} automatic={W.automatic} downloadable onPick={(choice) => pick({ use: 'speak', choice: choice || null })} />
        <UseLine {...shared} id="voice" label={W.speaker} row={view.voice} onPick={(choice) => pick({ use: 'voice', choice })} />
        <UseLine {...shared} id="hear" label={W.dictation} row={view.hear} onPick={() => {}} />
        <UseLine {...shared} id="draw" label={W.drawing} row={view.draw} automatic={view.draw.options.filter((o) => o.state === 'ready' || o.state === 'download').length >= 2 ? W.automatic : undefined} onPick={(choice) => pick({ use: 'draw', choice: choice || null })} downloadable />
        {!!view.apps.length && <div className="mt-3 rounded-lg bg-slate-50 px-3 py-2 dark:bg-white/5">
          <h3 className="text-xs font-bold text-slate-600 dark:text-slate-300">{W.appsModels}</h3>
          {view.apps.map((app) => <UseLine {...shared} key={app.id} id={`app-${app.id}`} label={app.label} row={{ ...view.chat, choice: app.choice, effective: null }} automatic={W.followChat} onPick={(choice) => pick({ use: 'app', id: app.id, choice: choice || null })} />)}
        </div>}
      </div>}
    </section>
  );
}
