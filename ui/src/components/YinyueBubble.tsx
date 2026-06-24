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
import { useUiStore } from '../stores/uiStore';

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
    <div className={`${pos} z-[9998] pointer-events-none`}>
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
      </div>
    </div>
  );
};
