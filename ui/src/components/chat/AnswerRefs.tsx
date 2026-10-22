// What an answer's numbers draw (lib/answerRefs.mts): a source as a link
// chip, pictures as captioned thumbnails in a grid that open larger on a
// click — the same viewer as a person's own pictures. Each reads its number
// from the session state's `refs`; a number with nothing behind it draws
// nothing, so a picture can never be made up.
import React, { useState } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { isPictureId, pictureCaption, sourceLabel, type AnswerRef } from '../../lib/answerRefs.mts';
import { Lightbox } from './ChatImages';
import { useImageSrc } from './useImageSrc';

function useAnswerRef(id: string): AnswerRef | undefined {
  return useChatStore((s) => s.sessionState?.refs?.[id]);
}

export const RefChip: React.FC<{ id: string }> = ({ id }) => {
  const ref = useAnswerRef(id);
  if (!ref || ref.kind !== 'source' || !ref.url) return null;
  return (
    <a
      href={ref.url}
      target="_blank"
      rel="noopener noreferrer"
      title={ref.title || ref.url}
      data-answer-ref={id}
      className="inline-flex items-center align-baseline mx-0.5 px-1.5 py-px rounded-full text-[11px] leading-4 no-underline bg-slate-100 text-slate-600 hover:bg-slate-200 dark:bg-white/10 dark:text-slate-300 dark:hover:bg-white/20"
    >
      {sourceLabel(ref)}
    </a>
  );
};

const Picture: React.FC<{ id: string; answer: AnswerRef }> = ({ id, answer }) => {
  const src = useImageSrc(answer.src || '');
  const [open, setOpen] = useState(false);
  const caption = pictureCaption(answer);
  const href = answer.page || answer.url || undefined;
  if (!src) return null;
  return (
    <figure className="m-0 flex flex-col gap-1 min-w-0" data-answer-ref={id}>
      <button type="button" onClick={() => setOpen(true)} className="p-0 border-0 bg-transparent cursor-zoom-in" title={caption || 'Open'}>
        <img src={src} alt={caption} loading="lazy" className="w-full h-32 object-cover rounded border border-slate-200 dark:border-white/10" />
      </button>
      {caption && <figcaption className="text-[11px] leading-4 text-slate-500 dark:text-slate-400 line-clamp-2">{caption}</figcaption>}
      {open && <Lightbox src={src} caption={caption} href={href} onClose={() => setOpen(false)} />}
    </figure>
  );
};

export const RefPictures: React.FC<{ ids: string }> = ({ ids }) => {
  const refs = useChatStore((s) => s.sessionState?.refs);
  const shown = ids
    .split(' ')
    .filter(isPictureId)
    .map((id) => [id, refs?.[id]] as const)
    .filter((pair): pair is readonly [string, AnswerRef] => !!pair[1] && pair[1].kind === 'picture' && !!pair[1].src);
  if (!shown.length) return null;
  return (
    <div className="grid grid-cols-2 sm:grid-cols-3 gap-2 my-2 not-prose" data-no-i18n>
      {shown.map(([id, ref]) => <Picture key={id} id={id} answer={ref} />)}
    </div>
  );
};
