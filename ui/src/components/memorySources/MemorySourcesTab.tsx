import React from 'react';
import { memorySourcesApi } from '../../lib/endpoints';
import {
  OWN_SOURCE, WORDS, countLine, fileCountLine,
  type Source,
} from '../../lib/memorySources.mts';
import { Section } from '../settings/kit';
import { OwnNotes } from './OwnNotes';
import { OtherAgentNotes } from './OtherAgentNotes';
import { useLoad } from './useLoad';

const tabCls = (on: boolean) =>
  `px-3 py-1.5 text-xs font-semibold rounded-lg border transition-colors ${
    on
      ? 'bg-blue-600/10 border-blue-500/40 text-blue-600 dark:text-blue-400'
      : 'border-slate-200 dark:border-white/10 text-slate-500 hover:border-slate-300 dark:hover:border-white/20'
  }`;

const sourceCount = (s: Source) => (s.own ? countLine(s.count) : fileCountLine(s.count));

/** Settings → Notes: one tab for each source of memory the engine lists. */
export const MemorySourcesTab: React.FC = () => {
  const sources = useLoad(() => memorySourcesApi.sources().then((r) => r.sources), []);
  const [picked, setPicked] = React.useState<string | null>(null);

  if (sources.error) {
    return <Section title={WORDS.title}><p role="alert" className="text-xs text-red-500">{WORDS.failed}</p></Section>;
  }
  if (!sources.value) {
    return <Section title={WORDS.title}><p className="text-xs text-slate-400">{WORDS.loading}</p></Section>;
  }
  const active = sources.value.find((s) => s.id === picked) ?? sources.value[0];
  return (
    <div className="space-y-4" data-testid="memory-sources">
      <Section title={WORDS.title} desc={WORDS.intro}>
        <div role="tablist" className="flex flex-wrap gap-2">
          {sources.value.map((s) => (
            <button key={s.id} role="tab" aria-selected={s.id === active.id} data-testid="memory-source-tab" data-source={s.id}
              className={tabCls(s.id === active.id)} onClick={() => setPicked(s.id)}>
              <span data-no-i18n>{s.label}</span> <span className="font-normal opacity-70">{sourceCount(s)}</span>
            </button>
          ))}
        </div>
        {active.unreadable && active.unreadable.length > 0 && (
          <p className="mt-3 text-[11px] text-amber-700 dark:text-amber-400">{WORDS.unreadable}: <span data-no-i18n>{active.unreadable.join(', ')}</span></p>
        )}
      </Section>
      {active.id === OWN_SOURCE
        ? <OwnNotes source={active} reloadSources={sources.reload} />
        : <OtherAgentNotes source={active} />}
    </div>
  );
};
