/**
 * Yinyue's VRM avatar — her embodiment in the web UI, docked at the bottom of
 * the right sidebar (see MainApp). Renders her VRM via `PetStage` (render-only)
 * and feeds the mouth opening from the audio handler's live signal each frame
 * so lip-sync stays in step with the voice it doesn't own; expression follows
 * the current speech's emotion.
 *
 * Click her to talk: a small input opens and the message is sent to the Yinyue
 * agent (`POST /api/yinyue/chat`); her reply comes back over the event spine
 * (she speaks + the bubble). The click also satisfies browser autoplay so her
 * voice can play.
 *
 * Loads `/yinyue.vrm` — Yinyue's original VRoid model (authored in-house).
 * Audio is independent of the model, so the voice still works if it fails.
 */
import React, { useEffect, useRef, useState } from 'react';
import { PetStage, type EmotionName, type ActionName, type SeqStep } from './PetStage';
import { loadIntents, pickClip } from './petActions';
import { getMouthOpening } from '../../lib/eventHandlers/yinyue';
import { _originalFetch } from '../../lib/fetchProxy';
import { useUiStore } from '../../stores/uiStore';

// Validate the wire strings against the renderer's vocabularies.
const EMOTION_NAMES: EmotionName[] = ['neutral', 'happy', 'angry', 'sad', 'relaxed'];
const ACTION_NAMES: ActionName[] = ['nod', 'shake', 'wave', 'bow', 'dance', 'cheer'];
const asEmotion = (e?: string): EmotionName | null =>
  (EMOTION_NAMES as string[]).includes(e ?? '') ? (e as EmotionName) : null;
const asAction = (a?: string): ActionName | null =>
  (ACTION_NAMES as string[]).includes(a ?? '') ? (a as ActionName) : null;

export const YinyueAvatar: React.FC = () => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const stageRef = useRef<PetStage | null>(null);
  const thinkTimer = useRef<number>(0);
  const express = useUiStore((s) => s.petExpress);
  const thinking = useUiStore((s) => s.petThinking);
  const speaking = useUiStore((s) => s.petSpeaking);
  const [composing, setComposing] = useState(false);
  const [draft, setDraft] = useState('');

  // Mount the renderer + drive the mouth from the live audio signal each frame.
  useEffect(() => {
    if (!canvasRef.current) return;
    const stage = new PetStage(canvasRef.current);
    stageRef.current = stage;
    stage.load('/yinyue.vrm').catch((e) => console.warn('[yinyue] avatar model load failed', e));

    let raf = 0;
    const tick = () => {
      raf = requestAnimationFrame(tick);
      stage.setMouthTarget(getMouthOpening());
    };
    raf = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(raf);
      stage.dispose();
      stageRef.current = null;
    };
  }, []);

  // Express tool → set emotion (sustained) once, then play the action(s) as a
  // (possibly single-step) sequence dispatched by render kind. `action` may be a
  // comma-joined chain (e.g. "wave,tilt_head,shrug") from Express's `sequence`.
  useEffect(() => {
    const stage = stageRef.current;
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
  }, [express?.id]);

  // A reply is in flight → hold the pondering pose until she responds.
  useEffect(() => {
    stageRef.current?.setThinking(thinking);
  }, [thinking]);

  // Her voice is playing → run the talking body loop.
  useEffect(() => {
    stageRef.current?.setSpeaking(speaking);
  }, [speaking]);

  async function send() {
    const text = draft.trim();
    setDraft('');
    setComposing(false);
    if (!text) return;
    const { setPetThinking } = useUiStore.getState();
    setPetThinking(true); // ponder until her reply arrives over the event spine
    window.clearTimeout(thinkTimer.current);
    thinkTimer.current = window.setTimeout(() => setPetThinking(false), 30000); // safety net
    try {
      await _originalFetch('/api/yinyue/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text }),
      });
      // Her reply arrives over the event spine → handlePetSpeak clears thinking.
    } catch (e) {
      console.error('[yinyue] chat failed', e);
      setPetThinking(false);
      window.clearTimeout(thinkTimer.current);
    }
  }

  return (
    <div className="relative h-full w-full">
      <canvas
        ref={canvasRef}
        className="h-full w-full cursor-pointer"
        title="Talk to Yinyue"
        onClick={() => setComposing((c) => !c)}
      />
      {composing && (
        <div className="absolute inset-x-2 bottom-2">
          <input
            autoFocus
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
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
