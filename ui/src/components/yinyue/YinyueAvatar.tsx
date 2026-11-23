/**
 * Yinyue's VRM avatar — her embodiment in the web UI, docked at the bottom of
 * the right sidebar (see MainApp). Renders her VRM via `PetStage` (render-only)
 * and feeds the mouth opening from the audio handler's live signal each frame
 * so lip-sync stays in step with the voice it doesn't own; expression follows
 * the current speech's emotion.
 *
 * Click her to type: a small input opens and the message is sent to the Yinyue
 * agent (`lib/askYinyue.ts`); her reply comes back over the event spine
 * (she speaks + the bubble). Hold her (or the space bar) to speak instead:
 * shared/hold-to-talk.js hears the line and sends it the same way, asked aloud. While a stage on an app page with a chat holds
 * her, the engine says it in that chat instead — as the user's `@银月 …` line,
 * answered there. The click also satisfies browser autoplay so her
 * voice can play.
 *
 * Loads `/yinyue.vrm` — Yinyue's original VRoid model (authored in-house).
 * Audio is independent of the model, so the voice still works if it fails.
 */
import React, { useEffect, useRef, useState } from 'react';
// Type-only: the renderer (three.js + three-vrm) is loaded on mount, in its own chunk.
import type { PetStage, EmotionName, ActionName, SeqStep } from './PetStage';
import { loadIntents, pickClip } from './petActions';
import { getMouthOpening } from '../../lib/eventHandlers/yinyue';
import { useUiStore } from '../../stores/uiStore';
import { postToParent } from '../../lib/parentFrame';
import { askYinyue } from '../../lib/askYinyue';
import { talk, useTalk } from '../../lib/yinyueTalk';
import { DRAG_PX, isTalkKey } from '@linggen/hold-to-talk';

// Validate the wire strings against the renderer's vocabularies.
const EMOTION_NAMES: EmotionName[] = ['neutral', 'happy', 'angry', 'sad', 'relaxed'];
const ACTION_NAMES: ActionName[] = ['nod', 'shake', 'wave', 'bow', 'dance', 'cheer'];
const asEmotion = (e?: string): EmotionName | null =>
  (EMOTION_NAMES as string[]).includes(e ?? '') ? (e as EmotionName) : null;
const asAction = (a?: string): ActionName | null =>
  (ACTION_NAMES as string[]).includes(a ?? '') ? (a as ActionName) : null;

/** The pet window has no page around her: the space bar is hers there, whoever
 *  has focus (a field keeps it). On the page it is hers only while she has focus. */
const IN_PET_WINDOW = new URLSearchParams(window.location.search).get('pet') === '1';

export const YinyueAvatar: React.FC = () => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const stageRef = useRef<PetStage | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const pressedAt = useRef<{ x: number; y: number } | null>(null);
  const swallowClick = useRef(false); // the click that ends a hold is not a tap
  const express = useUiStore((s) => s.petExpress);
  const thinking = useUiStore((s) => s.petThinking);
  const speaking = useUiStore((s) => s.petSpeaking);
  const [composing, setComposing] = useState(false);
  const [draft, setDraft] = useState('');
  const { view: heardView, label: talkState } = useTalk();

  // Mount the renderer + drive the mouth from the live audio signal each frame.
  // The renderer module (three.js) is imported here so it stays out of the
  // main bundle; `stage` state lets the effects below re-apply once it exists.
  const [stage, setStage] = useState<PetStage | null>(null);
  useEffect(() => {
    let cancelled = false;
    let mounted: PetStage | null = null;
    let raf = 0;
    // A page that stands her in a place (an iframe host) holds a placeholder
    // until she is DRAWN — the iframe's own load fires long before: the peer
    // connects, the presenter lock arrives, then the model loads. Tell it.
    const tellHost = (event: 'ready' | 'gone') => {
      postToParent({ type: 'linggen-pet', event });
    };
    import('./PetStage')
      .then(({ PetStage }) => {
        if (cancelled || !canvasRef.current) return;
        const s = new PetStage(canvasRef.current);
        mounted = s;
        stageRef.current = s;
        setStage(s);
        s
          .load('/yinyue.vrm')
          .then(() => { if (stageRef.current === s) tellHost('ready'); })
          .catch((e) => console.warn('[yinyue] avatar model load failed', e));
        const tick = () => {
          raf = requestAnimationFrame(tick);
          s.setMouthTarget(getMouthOpening());
        };
        raf = requestAnimationFrame(tick);
      })
      .catch((e) => console.warn('[yinyue] avatar renderer load failed', e));

    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
      if (mounted) {
        mounted.dispose();
        stageRef.current = null;
        tellHost('gone');
      }
    };
  }, []);

  // Express tool → set emotion (sustained) once, then play the action(s) as a
  // (possibly single-step) sequence dispatched by render kind. `action` may be a
  // comma-joined chain (e.g. "wave,tilt_head,shrug") from Express's `sequence`.
  useEffect(() => {
    if (!stage || !express) return;
    const { emotion, action } = express;
    void loadIntents().then((intents) => {
      if (stageRef.current !== stage) return; // remounted while loading
      const names = (action ?? '')
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean);

      // Emotion (sustained): explicit wins, else the first intent's default mood.
      const first = names.length ? intents.get(names[0]) : undefined;
      const e = asEmotion(emotion) ?? asEmotion(first?.emotion ?? undefined);
      if (e) stage.setEmotion(e);
      if (!names.length) return;

      const steps: SeqStep[] = [];
      for (const name of names) {
        const intent = intents.get(name);
        if (!intent) {
          // Back-compat: a bare action name → raw procedural gesture.
          const a = asAction(name);
          if (a) steps.push({ render: 'proc', proc: a });
          continue;
        }
        switch (intent.render) {
          case 'proc': {
            const a = asAction(intent.proc);
            if (a) steps.push({ render: 'proc', proc: a });
            break;
          }
          case 'visibility':
            steps.push({ render: 'visibility', visible: intent.visible !== false });
            break;
          case 'clip': {
            const url = pickClip(intent);
            if (url) steps.push({ render: 'clip', clipUrl: url, loop: intent.type === 'loop' });
            else console.info(`[pet] '${intent.name}' has no clip yet — emotion only`);
            break;
          }
        }
      }
      if (steps.length) void stage.playSequence(steps);
    });
    // Replays only when a new expression arrives (its id) or the renderer
    // first comes up, not when the store hands back an equal object.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [express?.id, stage]);

  // A reply is in flight → hold the pondering pose until she responds.
  useEffect(() => {
    stage?.setThinking(thinking);
  }, [stage, thinking]);

  // Her voice is playing → run the talking body loop.
  useEffect(() => {
    stage?.setSpeaking(speaking);
  }, [stage, speaking]);

  // Close the talk-input when the window loses focus — clicking another window
  // (app switch) doesn't fire the input's own blur in a webview, so it would
  // otherwise stay open.
  useEffect(() => {
    const close = () => setComposing(false);
    window.addEventListener('blur', close);
    return () => window.removeEventListener('blur', close);
  }, []);

  // Hold to talk (shared/hold-to-talk.js is the one control; this only binds it):
  // the space bar while she has focus (anywhere in the pet window), and the
  // press on her body below. Leaving ends a hold, so the microphone never stays.
  useEffect(() => {
    const target: HTMLElement | Window | null = IN_PET_WINDOW ? window : wrapRef.current;
    if (!target) return;
    const down = (e: Event) => {
      const k = e as KeyboardEvent;
      // A held key repeats: it is still hers (no scrolling the page), not a new press.
      if (!talk.offered()) return;
      if (k.repeat && isTalkKey({ code: k.code, repeat: false, metaKey: k.metaKey, ctrlKey: k.ctrlKey, altKey: k.altKey, target: k.target })) k.preventDefault();
      if (!isTalkKey(k)) return;
      k.preventDefault();
      talk.press('key');
    };
    const up = (e: Event) => {
      if ((e as KeyboardEvent).code === 'Space') talk.release('key');
    };
    target.addEventListener('keydown', down);
    target.addEventListener('keyup', up);
    return () => {
      target.removeEventListener('keydown', down);
      target.removeEventListener('keyup', up);
      talk.cancel();
    };
  }, []);

  function pressBody(e: React.PointerEvent<HTMLCanvasElement>) {
    if (e.button !== 0) return;
    pressedAt.current = { x: e.clientX, y: e.clientY };
    try { e.currentTarget.setPointerCapture(e.pointerId); } catch { /* a synthetic pointer: no capture */ }
    talk.press('pointer');
  }
  // Moving before the hold begins is a drag of the pet window, not a talk.
  function moveBody(e: React.PointerEvent<HTMLCanvasElement>) {
    const from = pressedAt.current;
    if (!from || talk.view().phase !== 'pressed') return;
    if (Math.hypot(e.clientX - from.x, e.clientY - from.y) > DRAG_PX) talk.cancel();
  }
  function releaseBody() {
    pressedAt.current = null;
    if (talk.release('pointer') !== 'talk') return;
    swallowClick.current = true;
    window.setTimeout(() => { swallowClick.current = false; }, 0);
  }

  async function send() {
    const text = draft.trim();
    setDraft('');
    setComposing(false);
    if (!text) return;
    await askYinyue(text);
  }

  // Click her to talk: the input opens + focuses; it hides again on blur. Her
  // body is also a drag handle in the pet window, so the input is data-no-drag.
  return (
    <div
      ref={wrapRef}
      tabIndex={0}
      className="relative h-full w-full outline-none"
      onBlur={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) talk.cancel(); }}
    >
      <canvas
        ref={canvasRef}
        className="h-full w-full cursor-pointer touch-none select-none"
        title="Talk to Yinyue"
        onPointerDown={pressBody}
        onPointerMove={moveBody}
        onPointerUp={releaseBody}
        onPointerCancel={() => { pressedAt.current = null; talk.cancel(); }}
        onClick={() => { if (!swallowClick.current) setComposing((c) => !c); }}
      />
      {!composing && (heardView.heard || heardView.note || talkState) && (
        <div data-testid="talk-status" className="pointer-events-none absolute inset-x-2 bottom-20 flex flex-col items-center gap-1 text-center">
          {heardView.heard && (
            <p data-testid="talk-heard" data-no-i18n className="rounded-lg bg-zinc-900/90 px-3 py-1 text-[12px] text-zinc-200 ring-1 ring-violet-400/30">
              “{heardView.heard}”
            </p>
          )}
          {(heardView.note ?? talkState) && (
            <p data-testid="talk-state" className="rounded-full bg-violet-500/80 px-3 py-0.5 text-[12px] font-medium text-white">
              {heardView.note ?? talkState}
            </p>
          )}
        </div>
      )}
      {composing && (
        <div className="absolute inset-x-2 bottom-20" data-no-drag>
          <input
            autoFocus
            data-no-drag
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              // IME composition: Enter here commits the candidate (e.g.
              // pinyin -> DJ), not the message. keyCode 229 covers Safari.
              if (e.nativeEvent.isComposing || e.keyCode === 229) return;
              if (e.key === 'Enter') send();
              else if (e.key === 'Escape') setComposing(false);
            }}
            onBlur={() => setComposing(false)}
            placeholder="Talk to Yinyue…"
            className="w-full rounded-lg bg-zinc-900/90 px-3 py-1.5 text-[13px] text-zinc-100 outline-none ring-1 ring-violet-400/40 placeholder:text-zinc-500"
          />
        </div>
      )}
    </div>
  );
};
