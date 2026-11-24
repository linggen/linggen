import type { ReactNode } from 'react';
import { ChevronDown } from 'lucide-react';
import { W, say } from '../../lib/modelsWords.mts';
import { ChatFallbacks } from './ChatFallbacks';
import type { UseEdit, UseOption, UseRow, Uses } from '../../lib/modelUses';
import { EFFORT_WORD } from '../../lib/effortChip.mts';
import type { Provider } from '../../lib/providers.mts';
import { ConnectionTag, cardCls, headingCls, hintCls, inputCls } from './shell';

const optionName = (option: UseOption) => ({
  qwen: 'Qwen3-TTS', system: W.systemVoice, 'local-stt': 'Whisper',
  ...EFFORT_WORD,
} as Record<string, string>)[option.id] ?? option.label;
const status = (option: UseOption) => ({
  ready: W.ready, no_key: W.liveMissingKey, signed_out: W.notSignedIn, unavailable: W.notReady,
  download: option.disk_gb ? say(W.downloadSize, { n: option.disk_gb }) : W.notDownloaded,
})[option.state];

/** Join the defaults snapshot to provider metadata for display only. */
function sourceOf(use: string, option: UseOption, providers: Provider[]) {
  if (use === 'chat' || use.startsWith('app-')) {
    return providers.find((p) => p.models.some((m) => m.kind === 'chat' && m.id === option.id));
  }
  if (use === 'live') return providers.find((p) => p.models.some((m) => m.id === `live:${option.id}`));
  if (use === 'draw' && option.id === 'chatgpt') return providers.find((p) => p.provider === 'chatgpt' && p.way === 'subscription');
  if ((use === 'speak' && option.id === 'qwen') || (use === 'draw' && option.id === 'local') || (use === 'hear' && option.id === 'local-stt')) {
    return providers.find((p) => p.way === 'local' && p.models.some((m) => m.kind === use && m.model === option.label));
  }
}

function UseLine({ id, label, purpose, row, providers, busy, onPick, automatic, downloadable = false, inline, note }: {
  id: string; label: string; purpose?: string; row: UseRow; providers: Provider[]; busy: boolean;
  automatic?: string; downloadable?: boolean; onPick: (choice: string) => void; inline?: ReactNode; note?: ReactNode;
}) {
  const canPick = (o: UseOption) => o.state === 'ready' || (downloadable && o.state === 'download');
  const choices = row.options.filter(canPick);
  const current = row.choice ?? (automatic ? '' : row.effective ?? '');
  const picker = id === 'chat' || choices.length + Number(!!automatic) >= 2;
  const selected = row.options.find((o) => o.id === (current || row.effective));
  const source = selected && sourceOf(id, selected, providers);
  const way = source?.way ?? (id === 'speak' && selected ? 'local' : undefined);
  const downloaded = way === 'local' && selected?.state === 'ready' && source?.disk_gb != null;
  const name = selected ? optionName(selected) : current || automatic || W.notSet;
  const followsAutomatic = picker && !!automatic && !current && !!selected;
  const provider = source?.label ?? (way === 'local' ? W.thisMac : undefined);
  const facts = (picker ? row.options.filter((o) => !canPick(o) || o.state !== 'ready') : row.options).filter((o) => o.id !== selected?.id);
  return (
    <div className="grid items-center gap-1.5 border-t border-slate-100 px-1 py-2.5 first:border-t-0 md:grid-cols-[minmax(7rem,130px)_minmax(0,1fr)_minmax(8rem,30%)] md:gap-x-3.5 dark:border-white/5">
      <div>
        <span className="text-[13px] font-semibold text-slate-700 dark:text-slate-200">{label}</span>
        {purpose && <p className={hintCls}>{purpose}</p>}
      </div>
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-1.5">
          {id === 'chat' && <label htmlFor="default-chat" className={`${hintCls} shrink-0`}>{W.chatDefault}</label>}
          <div className={`relative flex min-w-0 flex-1 basis-36 items-center gap-1.5 rounded-[10px] border px-2.5 py-1.5 text-xs focus-within:ring-2 focus-within:ring-blue-500/60 ${id === 'chat' ? 'border-amber-300 bg-amber-50/60 dark:border-amber-500/40 dark:bg-amber-500/10' : 'border-slate-200 bg-slate-50 dark:border-white/10 dark:bg-white/[0.03]'} ${!selected && !current ? 'border-dashed' : ''}`}>
            {way && <ConnectionTag way={way} />}
            <span className="min-w-0 truncate font-semibold text-slate-700 dark:text-slate-200" title={name}>{name}</span>
            {provider && <span className={`${hintCls} min-w-0 max-w-[38%] truncate`} title={provider}><span aria-hidden>· </span><span>{provider}</span></span>}
            {picker && <>
              <ChevronDown aria-hidden size={12} className="ml-auto shrink-0 text-slate-400" />
              {/* Native selection and keyboard behavior; the pill paints its closed state. */}
              <select id={`default-${id}`} data-testid={`default-${id}`} aria-label={id === 'chat' ? W.chatDefault : label} className="absolute inset-0 h-full w-full cursor-pointer opacity-0 disabled:cursor-default dark:[color-scheme:dark]" value={current} disabled={busy || choices.length === 0} onChange={(e) => onPick(e.target.value)}>
                {automatic && <option value="">{automatic}</option>}
                {!automatic && !current && <option value="" disabled>{W.notSet}</option>}
                {current && !row.options.some((o) => o.id === current) && <option value={current} disabled>{current}</option>}
                {row.options.map((option) => <option key={option.id} value={option.id} disabled={!canPick(option)}>{id === 'draw' && option.id === 'local' ? W.thisMac : optionName(option)}</option>)}
              </select>
            </>}
          </div>
          {inline}
        </div>
        {note}
        {followsAutomatic && <p className={`${hintCls} mt-1`}>{automatic}</p>}
        {!!facts.length && <div data-testid="use-line" className={`${hintCls} mt-1 flex flex-wrap gap-x-3 gap-y-0.5`}>
          {facts.map((o) => <span key={o.id}>{optionName(o)} <span>· {status(o)}</span></span>)}
        </div>}
      </div>
      <div className={`${hintCls} flex flex-wrap items-center gap-x-1.5 md:justify-end md:text-right`}>
        {downloaded ? <>
          <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
          <span>{W.downloaded}</span><span aria-hidden>·</span>
          <strong className="font-semibold text-slate-700 dark:text-slate-300">{say(W.gb, { n: source!.disk_gb! })}</strong>
          <span aria-hidden>·</span><span>{W.free}</span>
        </> : selected?.state === 'ready' && source?.way === 'key' ? <span>{say(W.billedBy, { provider: source.label })}</span>
          : selected?.state === 'ready' && way === 'local' ? <span>{W.free}</span>
          : selected?.state === 'ready' && source?.way === 'subscription' ? <span>{W.chatgptNote}</span>
          : selected?.state === 'ready' && source?.way === 'account' ? <span>{W.cloudNote}</span>
          : selected && <span>{status(selected)}</span>}
      </div>
    </div>
  );
}

function Effort({ row, busy, onPick }: { row: UseRow; busy: boolean; onPick: (choice: string) => void }) {
  if (!row.options.length) return null;
  const current = row.options.some((o) => o.id === row.choice) ? row.choice! : '';
  return <label className={`${hintCls} flex shrink-0 items-center gap-1.5`}>
    <span>{W.thinking}</span>
    <select data-testid="default-effort" aria-label={W.effort} className={`${inputCls} px-2 py-1 dark:[color-scheme:dark]`} value={current} disabled={busy} onChange={(e) => onPick(e.target.value || 'auto')}>
      <option value="">{W.automatic}</option>
      {row.options.map((o) => <option key={o.id} value={o.id}>{optionName(o)}</option>)}
    </select>
  </label>;
}

export function DefaultCard({ view, providers, busy, failed, retry, pick }: {
  view: Uses | null; providers: Provider[]; busy: boolean; failed: boolean; retry: () => void; pick: (edit: UseEdit) => void;
}) {
  const shared = { busy, providers };
  const liveCanPick = (view?.live.options.filter((o) => o.state === 'ready').length ?? 0) >= 2;
  return (
    <section className={cardCls} data-testid="default-card">
      <h2 className={headingCls}>{W.defaultModel}</h2>
      <p className={`${hintCls} mt-1`}>{W.defaultModelHint}</p>
      {failed && <p role="alert" className="mt-2 text-xs text-red-600 dark:text-red-400">{W.defaultsFailed} <button type="button" className="underline" onClick={retry}>{W.retry}</button></p>}
      {!view && !failed && <p className={`${hintCls} mt-2`}>{W.loading}</p>}
      {view && <div className="mt-3">
        <UseLine {...shared} id="chat" label={W.chat} purpose={W.chatPurpose} row={view.chat} onPick={(choice) => pick({ use: 'chat', choice })}
          inline={<Effort row={view.effort} busy={busy} onPick={(choice) => pick({ use: 'effort', choice })} />}
          note={<ChatFallbacks view={view} busy={busy} pick={pick} />} />
        <UseLine {...shared} id="live" label={W.liveVoice} purpose={W.livePurpose} row={view.live} automatic={liveCanPick ? W.automatic : undefined} onPick={(choice) => pick({ use: 'live', choice: choice || null })} />
        <UseLine {...shared} id="speak" label={W.readAloud} purpose={W.speakPurpose} row={view.speak} automatic={W.automatic} downloadable onPick={(choice) => pick({ use: 'speak', choice: choice || null })}
          note={view.voice.options.length >= 2 && <label className={`${hintCls} mt-1 flex flex-wrap items-center gap-1.5`}>
            <span>{W.speaker}</span>
            <select data-testid="default-voice" className={`${inputCls} px-2 py-1 dark:[color-scheme:dark]`} value={view.voice.choice ?? ''} disabled={busy} onChange={(e) => pick({ use: 'voice', choice: e.target.value })}>
              {!view.voice.choice && <option value="" disabled>{W.notSet}</option>}
              {view.voice.choice && !view.voice.options.some((o) => o.id === view.voice.choice) && <option value={view.voice.choice} disabled>{view.voice.choice}</option>}
              {view.voice.options.map((o) => <option key={o.id} value={o.id}>{optionName(o)}</option>)}
            </select>
          </label>} />
        <UseLine {...shared} id="hear" label={W.dictation} purpose={W.hearPurpose} row={view.hear} onPick={() => {}} />
        <UseLine {...shared} id="draw" label={W.drawing} purpose={W.drawPurpose} row={view.draw} automatic={view.draw.options.filter((o) => o.state === 'ready' || o.state === 'download').length >= 2 ? W.automatic : undefined} onPick={(choice) => pick({ use: 'draw', choice: choice || null })} downloadable />
        {!!view.apps.length && <div className="mt-3 rounded-lg bg-slate-50 px-3 py-2 dark:bg-white/5">
          <h3 className="text-xs font-semibold text-slate-600 dark:text-slate-300">{W.appsModels}</h3>
          <div>{view.apps.map((app) => <UseLine {...shared} key={app.id} id={`app-${app.id}`} label={app.label} row={{ ...view.chat, choice: app.choice, effective: null }} automatic={W.followChat} onPick={(choice) => pick({ use: 'app', id: app.id, choice: choice || null })} />)}</div>
        </div>}
      </div>}
    </section>
  );
}
