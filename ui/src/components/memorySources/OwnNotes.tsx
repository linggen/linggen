import React from 'react';
import { memorySourcesApi } from '../../lib/endpoints';
import {
  REVIEW_TAB, WORDS, reviewLine,
  type Source,
} from '../../lib/memorySources.mts';
import { Section } from '../settings/kit';
import { NoteRowView } from './NoteRowView';
import { useLoad } from './useLoad';
import { useNoteActions } from './useNoteActions';

const itemCls = (on: boolean) =>
  `w-full text-left rounded-lg px-3 py-2 text-xs border ${
    on ? 'border-blue-500/40 bg-blue-600/10' : 'border-transparent hover:bg-slate-100 dark:hover:bg-white/5'
  }`;

/** Linggen's own notes: topics, a topic's notes, and the review list — the only notes that can be changed. */
export const OwnNotes: React.FC<{ source: Source; reloadSources: () => Promise<void> }> = ({ source, reloadSources }) => {
  const topics = useLoad(() => memorySourcesApi.topics(source.id).then((r) => r.topics), [source.id]);
  const [picked, setPicked] = React.useState<string>(REVIEW_TAB);
  const topic = picked === REVIEW_TAB ? null : picked;
  const notes = useLoad(
    () => (topic ? memorySourcesApi.ownNotes(topic).then((r) => r.entries) : Promise.resolve(null)),
    [topic],
  );
  // A topic whose last note went is no longer there: back to the review list.
  React.useEffect(() => { if (topic && notes.error) setPicked(REVIEW_TAB); }, [topic, notes.error]);
  const review = useLoad(() => memorySourcesApi.review().then((r) => r.review), []);

  // After a change the engine is asked again for everything the page shows.
  const reload = React.useCallback(async () => {
    await Promise.all([topics.reload(), notes.reload(), review.reload(), reloadSources()]);
  }, [topics, notes, review, reloadSources]);
  const { run, dialog, error } = useNoteActions(reload);

  const reviewCount = review.value?.length ?? source.review ?? 0;
  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-[16rem_1fr]">
      <Section title={WORDS.topics}>
        <button className={itemCls(picked === REVIEW_TAB)} data-testid="memory-review-tab" onClick={() => setPicked(REVIEW_TAB)}>
          {WORDS.review} <span className="opacity-60">{reviewCount}</span>
        </button>
        {topics.value?.length === 0 && <p className="mt-2 text-[11px] text-slate-400">{WORDS.noTopics}</p>}
        <ul className="mt-1 space-y-0.5">
          {topics.value?.map((t) => (
            <li key={t.path}>
              <button className={itemCls(picked === t.path)} data-testid="memory-topic" data-path={t.path} onClick={() => setPicked(t.path)}>
                <span data-no-i18n>{t.name}</span> <span className="opacity-60">{t.count}</span>
                <span className="block text-[10px] text-slate-400" data-no-i18n>{t.scope}</span>
              </button>
            </li>
          ))}
        </ul>
      </Section>
      <Section title={topic ? (topics.value?.find((t) => t.path === topic)?.name ?? topic) : WORDS.review}
        desc={topic ? undefined : WORDS.reviewHint}>
        {error && <p role="alert" className="mb-2 text-xs text-red-500" data-testid="memory-error">{error}</p>}
        {topic
          ? <NoteList notes={notes.value} empty={WORDS.noEntries} onAction={run} />
          : (
            <ul data-testid="memory-review">
              {review.value?.length === 0 && <p className="text-xs text-slate-500">{WORDS.reviewEmpty}</p>}
              {review.value?.map((n) => <NoteRowView key={n.id} note={n} extra={reviewLine(n)} onAction={run} />)}
            </ul>
          )}
      </Section>
      {dialog}
    </div>
  );
};

const NoteList: React.FC<{
  notes: React.ComponentProps<typeof NoteRowView>['note'][] | null;
  empty: string;
  onAction: React.ComponentProps<typeof NoteRowView>['onAction'];
}> = ({ notes, empty, onAction }) => {
  if (!notes) return <p className="text-xs text-slate-400">{WORDS.loading}</p>;
  if (notes.length === 0) return <p className="text-xs text-slate-500">{empty}</p>;
  return <ul>{notes.map((n) => <NoteRowView key={n.id} note={n} onAction={onAction} />)}</ul>;
};

