import type { ReactNode } from 'react';
import { ChevronDown } from 'lucide-react';
import { W, say } from '../../lib/modelsWords.mts';
import { ChatFallbacks } from './ChatFallbacks';
import type { UseEdit, UseOption, UseRow, Uses } from '../../lib/modelUses';
import { EffortSlider } from './EffortSlider';
import { translate } from '../../lib/i18n';
import { ModelSourceTag, modelName, nativeModelLabel, sourceText } from './UseOptionLabel';
import { cardCls, headingCls, hintCls, inputCls } from './shell';

const status = (option: UseOption) => ({
  ready: W.ready, no_key: W.liveMissingKey, signed_out: W.notSignedIn,
  unavailable: W.notReady, download: W.notDownloaded,
})[option.state];

function UseLine({ id, label, purpose, row, busy, onPick, automatic, downloadable = false, inline, note }: {
  id: string; label: string; purpose?: string; row: UseRow; busy: boolean;
  automatic?: string; downloadable?: boolean; onPick: (choice: string) => void; inline?: ReactNode; note?: ReactNode;
}) {
  const canPick = (o: UseOption) => o.state === 'ready' || (downloadable && o.state === 'download');
  const choices = row.options.filter(canPick);
  const current = row.choice ?? (automatic ? '' : row.effective ?? '');
  const picker = id === 'chat' || choices.length + Number(!!automatic) >= 2;
  const effective = row.options.find((o) => o.id === row.effective);
  const selected = current ? row.options.find((o) => o.id === current) : effective;
  const source = selected?.source;
  const autoName = automatic === W.automatic && effective
    ? say(translate(W.automaticNow) ?? W.automaticNow, { model: modelName(effective) }) : automatic;
  const name = (!current && autoName) || (selected ? modelName(selected) : current ? W.modelMissing : W.notSet);
  const facts = (picker ? row.options.filter((o) => !canPick(o) || o.state !== 'ready') : row.options).filter((o) => o.id !== selected?.id);
  return (
    <div className="grid items-center gap-1.5 border-t border-slate-100 px-1 py-2.5 first:border-t-0 md:grid-cols-[minmax(7rem,130px)_minmax(0,1fr)_minmax(8rem,25%)] md:gap-x-3.5 dark:border-white/5">
      <div>
        <span className="text-[13px] font-semibold text-slate-700 dark:text-slate-200">{label}</span>
        {purpose && <p className={hintCls}>{purpose}</p>}
      </div>
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-1.5">
          {id === 'chat' && <label htmlFor="default-chat" className={`${hintCls} shrink-0`}>{W.chatDefault}</label>}
          <div className={`relative flex min-w-0 flex-1 basis-36 items-center gap-1.5 rounded-[10px] border px-2.5 py-1.5 text-xs focus-within:ring-2 focus-within:ring-blue-500/60 ${id === 'chat' ? 'border-amber-300 bg-amber-50/60 dark:border-amber-500/40 dark:bg-amber-500/10' : 'border-slate-200 bg-slate-50 dark:border-white/10 dark:bg-white/[0.03]'} ${!selected && !current ? 'border-dashed' : ''}`}>
            <span className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5">
              <span className="min-w-0 break-words font-semibold text-slate-700 dark:text-slate-200" title={selected?.model_id ?? (selected ? name : current || undefined)}>{name}</span>
              {selected && <ModelSourceTag option={selected} />}
            </span>
            {picker && <>
              <ChevronDown aria-hidden size={12} className="ml-auto shrink-0 text-slate-400" />
              {/* Native selection and keyboard behavior; the pill paints its closed state. */}
              <select id={`default-${id}`} data-testid={`default-${id}`} aria-label={id === 'chat' ? W.chatDefault : label} title={selected?.model_id} className="absolute inset-0 h-full w-full cursor-pointer opacity-0 disabled:cursor-default dark:[color-scheme:dark]" value={current} disabled={busy || choices.length === 0} onChange={(e) => onPick(e.target.value)}>
                {automatic && <option value="">{autoName}{effective && automatic === W.automatic ? ` · ${sourceText(effective)}` : ''}</option>}
                {!automatic && !current && <option value="" disabled>{W.notSet}</option>}
                {current && !selected && <option value={current} title={current} disabled>{W.modelMissing}</option>}
                {row.options.map((option) => <option key={option.id} title={option.model_id} value={option.id} disabled={!canPick(option)}>{nativeModelLabel(option)}</option>)}
              </select>
            </>}
          </div>
          {inline && <div className="basis-full md:basis-auto">{inline}</div>}
        </div>
        {note}
        {!!facts.length && <div data-testid="use-line" className={`${hintCls} mt-1 flex flex-wrap gap-x-3 gap-y-0.5`}>
          {facts.map((o) => <span key={o.id} className="inline-flex flex-wrap items-center gap-1" title={o.model_id}>
            <span>{modelName(o)}</span><ModelSourceTag option={o} /><span>· </span><span>{status(o)}</span>
          </span>)}
        </div>}
      </div>
      <div className={`${hintCls} flex flex-wrap items-center gap-x-1.5 md:justify-end md:text-right`}>
        {selected?.state === 'ready' && source?.way === 'local' && source.size_gb != null ? <>
          <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
          <span>{W.downloaded}</span>
        </> : selected?.state === 'ready' && source?.way === 'key' && source.provider ? <span>{say(W.billedBy, { provider: source.provider })}</span>
          : selected?.state === 'ready' && source?.way === 'local' ? null
          : selected?.state === 'ready' && source?.way === 'subscription' ? <span>{W.chatgptNote}</span>
          : selected?.state === 'ready' && source?.way === 'account' ? <span>{W.cloudNote}</span>
          : selected && <span>{status(selected)}</span>}
      </div>
    </div>
  );
}

function Effort({ view, busy, pick }: { view: Uses; busy: boolean; pick: (edit: UseEdit) => void }) {
  const model = view.chat.options.find((option) => option.id === view.chat.choice);
  if (!model) return null;
  const row = view.effort[model.id];
  if (!row) return null;
  return <EffortSlider id="default-effort" name={modelName(model)} values={row.options.map((o) => o.id)} choice={row.choice} disabled={busy} onPick={(choice) => pick({ use: 'effort', model: model.id, choice })} />;
}

export function DefaultCard({ view, busy, failed, retry, pick }: {
  view: Uses | null; busy: boolean; failed: boolean; retry: () => void; pick: (edit: UseEdit) => void;
}) {
  return (
    <section className={cardCls} data-testid="default-card">
      <h2 className={headingCls}>{W.defaultModel}</h2>
      <p className={`${hintCls} mt-1`}>{W.defaultModelHint}</p>
      {failed && <p role="alert" className="mt-2 text-xs text-red-600 dark:text-red-400">{W.defaultsFailed} <button type="button" className="underline" onClick={retry}>{W.retry}</button></p>}
      {!view && !failed && <p className={`${hintCls} mt-2`}>{W.loading}</p>}
      {view && <div className="mt-3">
        <UseLine busy={busy} id="chat" label={W.chat} purpose={W.chatPurpose} row={view.chat} onPick={(choice) => pick({ use: 'chat', choice })}
          inline={<Effort view={view} busy={busy} pick={pick} />}
          note={<ChatFallbacks view={view} busy={busy} pick={pick} />} />
        <UseLine busy={busy} id="speak" label={W.readAloud} purpose={W.speakPurpose} row={view.speak} automatic={W.automatic} downloadable onPick={(choice) => pick({ use: 'speak', choice: choice || null })}
          note={view.voice.options.length >= 2 && view.speak.effective !== 'linggen-cloud' && <label className={`${hintCls} mt-1 flex flex-wrap items-center gap-1.5`}>
            <span>{W.speaker}</span>
            <select data-testid="default-voice" className={`${inputCls} px-2 py-1 dark:[color-scheme:dark]`} value={view.voice.choice ?? ''} disabled={busy} onChange={(e) => pick({ use: 'voice', choice: e.target.value })}>
              {!view.voice.choice && <option value="" disabled>{W.notSet}</option>}
              {view.voice.choice && !view.voice.options.some((o) => o.id === view.voice.choice) && <option value={view.voice.choice} disabled>{view.voice.choice}</option>}
              {view.voice.options.map((o) => <option key={o.id} value={o.id}>{nativeModelLabel(o)}</option>)}
            </select>
          </label>} />
        <UseLine busy={busy} id="hear" label={W.dictation} purpose={W.hearPurpose} row={view.hear} onPick={() => {}} />
        <UseLine busy={busy} id="draw" label={W.drawing} purpose={W.drawPurpose} row={view.draw} automatic={W.automatic} onPick={(choice) => pick({ use: 'draw', choice: choice || null })} downloadable />

      </div>}
    </section>
  );
}
