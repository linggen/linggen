/**
 * Yinyue's speech bubble — the web surface's visual layer for the
 * `yinyue_speak` event (paired with her voice in `eventHandlers/yinyue.ts`).
 *
 * A small, in-character card: her name in her signature violet, an emotion
 * accent (a dot + ring, no emoji — her persona avoids emoji spam), and the
 * spoken line. Click to dismiss; otherwise it lingers ~as long as the audio.
 * The full VRM avatar is a separate, larger slice; this is the lightweight
 * visual that ships with the voice.
 */
import React from 'react';
import { useUiStore, type SpeechRaised } from '../stores/uiStore';
import { needsConfirm, offerRequest, outcomeOf } from '../lib/petOffers.mts';
import { openApp } from '../lib/appHost';
import type { OfferButton } from '../types/generated/OfferButton';
import type { AskUserAnswer, PendingAskUser } from '../types';
import { PendingAskCard } from './PendingAskCard';
import { useInteractionStore } from '../stores/interactionStore';
import { sessionApi } from '../lib/endpoints';
import { listedAsk } from '../lib/askQueue.mts';
import { answerAsk } from '../lib/answerAsk';

// Violet is her signature (silver-lavender); emotions tint from there.
const ACCENTS: Record<string, { ring: string; dot: string }> = {
  happy: { ring: 'ring-emerald-400/40', dot: 'bg-emerald-400' },
  neutral: { ring: 'ring-violet-400/40', dot: 'bg-violet-400' },
  worried: { ring: 'ring-amber-400/40', dot: 'bg-amber-400' },
  alert: { ring: 'ring-rose-400/50', dot: 'bg-rose-400' },
};

export const YinyueBubble: React.FC<{ variant?: 'overlay' | 'pet' }> = ({ variant = 'overlay' }) => {
  const speech = useUiStore((s) => s.yinyueSpeech);
  const clear = useUiStore((s) => s.clearYinyueSpeech);
  if (!speech) return null;

  const accent = ACCENTS[speech.emotion] ?? ACCENTS.neutral;

  // `pet` = the narrow transparent pet window (PetApp): float above her head,
  // fit the window width. `overlay` = the in-page dock corner (MainApp).
  const pos = variant === 'pet'
    ? 'fixed top-3 inset-x-2 max-w-full'
    : 'fixed bottom-[19rem] right-4 max-w-xs';

  return (
    <div data-testid="yinyue-bubble" className={`${pos} z-[9998] pointer-events-none`}>
      <div
        onClick={clear}
        title="Dismiss"
        className={`pointer-events-auto cursor-pointer rounded-2xl rounded-br-sm bg-zinc-900/90 backdrop-blur px-4 py-3 shadow-xl ring-1 ${accent.ring} animate-slide-in`}
      >
        <div className="mb-1 flex items-center gap-1.5">
          <span className={`h-1.5 w-1.5 rounded-full ${accent.dot}`} />
          <span className="text-[11px] font-semibold tracking-wide text-violet-300">Yinyue</span>
        </div>
        <p className="text-[13px] leading-snug text-zinc-100">{speech.text}</p>
        {speech.raised && <Offers raised={speech.raised} done={clear} />}
        {speech.questionId && <WaitingCard questionId={speech.questionId} done={clear} />}
      </div>
    </div>
  );
};

/** The waiting question she told of, answered here as on the page: its own
 *  card (`PendingAskCard`) and the page's own answer (`answerAsk`), found
 *  among this window's questions or the engine's list. Answered anywhere,
 *  the card goes: the window's questions drop it, and the list is read
 *  again while the card shows. A failed answer keeps the card up. */
const WaitingCard: React.FC<{ questionId: string; done: () => void }> = ({ questionId, done }) => {
  const queues = useInteractionStore((s) => s.askQueues);
  const shown = React.useMemo(() => findAsk(queues, questionId), [queues, questionId]);
  const [listed, setListed] = React.useState<{ ask: PendingAskUser; sessionId: string | null } | null>(null);
  React.useEffect(() => {
    if (shown) return;
    let live = true;
    const read = () => sessionApi.pendingAskUser()
      .then((items) => {
        const it = items.find((i) => i.question_id === questionId);
        if (live) setListed(it ? { ask: listedAsk(it), sessionId: it.session_id ?? null } : null);
      })
      .catch(() => undefined);
    void read();
    const timer = window.setInterval(() => void read(), LISTED_REREAD_MS);
    return () => { live = false; window.clearInterval(timer); };
  }, [questionId, shown]);
  const found = shown ?? listed;
  if (!found) return null;
  const respond = async (id: string, answers: AskUserAnswer[]) => {
    if (await answerAsk(found.sessionId, id, answers)) done();
  };
  return (
    <div className="mt-2 max-w-sm text-left" onClick={(e) => e.stopPropagation()}>
      <PendingAskCard pending={{ ...found.ask, toldAloud: undefined }} onRespond={(id, answers) => void respond(id, answers)} />
    </div>
  );
};

/** How often a card read from the engine's list is checked still waiting. */
const LISTED_REREAD_MS = 3000;

/** A waiting question this window already holds, with its session. */
function findAsk(queues: Record<string, PendingAskUser[]>, questionId: string): { ask: PendingAskUser; sessionId: string } | null {
  for (const [sessionId, asks] of Object.entries(queues)) {
    const ask = asks.find((a) => a.questionId === questionId);
    if (ask) return { ask, sessionId };
  }
  return null;
}

/** The person answered: kept on the problem's record (and, for a tap, her
 *  result turn — the engine's). */
async function answer(problem: string, answer: string, result?: unknown, label?: string): Promise<void> {
  await fetch('/api/yinyue/answer', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ problem, answer, result, label }),
  }).catch(() => undefined);
}

/** Run one offer as the person's own tap — the skill-tool route a page
 *  button uses, or the app launch route for a place. */
async function runOffer(o: OfferButton): Promise<{ ok: boolean; result: unknown }> {
  const { url, body } = offerRequest(o);
  try {
    const resp = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    const text = await resp.text();
    let result: unknown = text;
    try { result = JSON.parse(text); } catch { /* plain text refusal */ }
    if (resp.ok && o.kind === 'open') openLaunched(o.skill, result);
    return { ok: resp.ok, result };
  } catch (e) {
    return { ok: false, result: String(e) };
  }
}

/** The page an offer launched, opened where this window opens apps (lib/appHost.ts).
 *  In the desktop app her bubble is in her own window, which leaves it to the
 *  main window's `app_launched`. */
function openLaunched(skill: string, result: unknown): void {
  const url = (result as { url?: string } | null)?.url;
  if (url) openApp({ skill, url });
}

/** Her offers as buttons under her labels, and ours: "Not now". A tool that
 *  changes anything asks once more before it runs. */
const Offers: React.FC<{ raised: SpeechRaised; done: () => void }> = ({ raised, done }) => {
  const [asking, setAsking] = React.useState<number | null>(null);
  const [busy, setBusy] = React.useState(false);
  const tap = async (i: number, o: OfferButton) => {
    if (busy) return;
    if (needsConfirm(o) && asking !== i) {
      setAsking(i);
      return;
    }
    setBusy(true);
    const { ok, result } = await runOffer(o);
    await answer(raised.problem, outcomeOf(o, ok, result), result, o.label);
    done();
  };
  const stop = (e: React.MouseEvent) => e.stopPropagation();
  return (
    <div className="mt-2 flex flex-wrap items-center gap-1.5" onClick={stop}>
      {raised.offers.map((o, i) => (
        <button
          key={`${o.kind}-${o.skill}-${o.tool ?? o.view ?? ''}`}
          disabled={busy}
          onClick={() => void tap(i, o)}
          className="rounded-full bg-violet-500/80 px-3 py-1 text-[12px] font-medium text-white hover:bg-violet-500 disabled:opacity-50"
        >
          {asking === i ? `${o.label} ✓?` : o.label}
        </button>
      ))}
      {/* The other device's problem is answered where it lives. */}
      {!raised.problem.includes('/') && <button
        disabled={busy}
        onClick={() => {
          void answer(raised.problem, 'not_now');
          done();
        }}
        className="rounded-full px-2 py-1 text-[12px] text-zinc-400 hover:text-zinc-200"
      >
        Not now
      </button>}
    </div>
  );
};
