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
 * Dev note: loads `/yinyue.vrm` (Keqing placeholder — gitignored, dev-only).
 * Audio is independent of the model, so the voice still works if it fails.
 */
import React, { useEffect, useRef, useState } from 'react';
import { PetStage, type EmotionName, type ActionName } from './PetStage';
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
  const express = useUiStore((s) => s.petExpress);
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

  // Express tool → drive emotion (sustained) + a one-shot gesture on the avatar.
  useEffect(() => {
    const stage = stageRef.current;
    if (!stage || !express) return;
    const e = asEmotion(express.emotion);
    if (e) stage.setEmotion(e);
    const a = asAction(express.action);
    if (a) stage.playAction(a);
  }, [express?.id]);

  async function send() {
    const text = draft.trim();
    setDraft('');
    setComposing(false);
    if (!text) return;
    try {
      await _originalFetch('/api/yinyue/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text }),
      });
      // Her reply arrives over the event spine → she speaks + the bubble shows.
    } catch (e) {
      console.error('[yinyue] chat failed', e);
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
