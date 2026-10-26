// What an answer's numbers draw (lib/answerRefs.mts): a source as a link
// chip, pictures as captioned thumbnails in a grid that open larger on a
// click — the same viewer as a person's own pictures — and a video as a card
// (its still, title, channel and length) that plays in place on a click. Each reads its number
// from the session state's `refs`; a number with nothing behind it draws
// nothing, so a picture can never be made up.
import React, { useState } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { isShownId, pictureCaption, sourceLabel, videoByline, type AnswerRef } from '../../lib/answerRefs.mts';
import { Lightbox } from './ChatImages';
import { useImageSrc } from './useImageSrc';

function useAnswerRef(id: string): AnswerRef | undefined {
  return useChatStore((s) => s.sessionState?.refs?.[id]);
}

/** Where a picture's number stood: a small thumbnail, opening it large. */
const InlinePicture: React.FC<{ id: string; answer: AnswerRef }> = ({ id, answer }) => {
  const src = useImageSrc(answer.src || '');
  const [open, setOpen] = useState(false);
  const caption = pictureCaption(answer);
  if (!src) return null;
  return (
    <>
      <button type="button" onClick={() => setOpen(true)} data-answer-ref={id} title={caption || 'Open'} className="inline-block align-text-bottom mx-0.5 p-0 border-0 bg-transparent cursor-zoom-in">
        <img src={src} alt={caption} className="h-5 w-7 object-cover rounded-sm border border-slate-200 dark:border-white/10" />
      </button>
      {open && <Lightbox src={src} caption={caption} href={answer.page || answer.url || undefined} onClose={() => setOpen(false)} />}
    </>
  );
};

/** Where a video's number stood: its still with a play mark, opening the video. */
const InlineVideo: React.FC<{ id: string; answer: AnswerRef }> = ({ id, answer }) => {
  const src = useImageSrc(answer.src || '');
  if (!src) return null;
  return (
    <a href={answer.url} target="_blank" rel="noopener noreferrer" data-answer-ref={id} title={answer.title || 'Video'} className="relative inline-block align-text-bottom mx-0.5 no-underline">
      <img src={src} alt={answer.title || ''} className="h-5 w-8 object-cover rounded-sm border border-slate-200 dark:border-white/10" />
      <span className="absolute inset-0 flex items-center justify-center text-[9px] text-white drop-shadow">▶</span>
    </a>
  );
};

export const RefChip: React.FC<{ id: string }> = ({ id }) => {
  const ref = useAnswerRef(id);
  if (ref?.kind === 'picture' && ref.src) return <InlinePicture id={id} answer={ref} />;
  if (ref?.kind === 'video' && ref.src) return <InlineVideo id={id} answer={ref} />;
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

/** A video's still with its length; a click plays it here (or opens it
 *  where it cannot play in a page). */
const Video: React.FC<{ id: string; answer: AnswerRef }> = ({ id, answer }) => {
  const src = useImageSrc(answer.src || '');
  const [playing, setPlaying] = useState(false);
  const byline = videoByline(answer);
  if (!src) return null;
  const play = () => (answer.embed ? setPlaying(true) : window.open(answer.url, '_blank', 'noopener'));
  return (
    <figure className={`m-0 flex flex-col gap-1 min-w-0 ${playing ? 'col-span-2 sm:col-span-3' : ''}`} data-answer-ref={id}>
      {playing ? (
        <iframe
          src={`${answer.embed}?autoplay=1&rel=0`}
          title={answer.title || 'Video'}
          allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
          allowFullScreen
          className="w-full aspect-video rounded border-0"
        />
      ) : (
        <button type="button" onClick={play} className="relative p-0 border-0 bg-transparent cursor-pointer" title={answer.title || 'Play'}>
          <img src={src} alt={answer.title || ''} loading="lazy" className="w-full h-32 object-cover rounded border border-slate-200 dark:border-white/10" />
          <span className="absolute inset-0 flex items-center justify-center">
            <span className="w-10 h-10 rounded-full bg-black/60 text-white flex items-center justify-center text-sm">▶</span>
          </span>
          {answer.duration && <span className="absolute bottom-1 right-1 px-1 rounded bg-black/70 text-white text-[10px] leading-4">{answer.duration}</span>}
        </button>
      )}
      <figcaption className="text-[11px] leading-4 text-slate-500 dark:text-slate-400">
        <a href={answer.url} target="_blank" rel="noopener noreferrer" className="line-clamp-2 text-slate-700 dark:text-slate-200 no-underline hover:underline">{answer.title}</a>
        {byline && <span className="block truncate">{byline}</span>}
      </figcaption>
    </figure>
  );
};

const drawable = (ref: AnswerRef | undefined): ref is AnswerRef => !!ref && (ref.kind === 'picture' || ref.kind === 'video') && !!ref.src;

export const RefPictures: React.FC<{ ids: string }> = ({ ids }) => {
  const refs = useChatStore((s) => s.sessionState?.refs);
  const shown = ids
    .split(' ')
    .filter(isShownId)
    .map((id) => [id, refs?.[id]] as const)
    .filter((pair): pair is readonly [string, AnswerRef] => drawable(pair[1]));
  if (!shown.length) return null;
  return (
    <div className="grid grid-cols-2 sm:grid-cols-3 gap-2 my-2 not-prose" data-no-i18n>
      {shown.map(([id, ref]) => (ref.kind === 'video' ? <Video key={id} id={id} answer={ref} /> : <Picture key={id} id={id} answer={ref} />))}
    </div>
  );
};
