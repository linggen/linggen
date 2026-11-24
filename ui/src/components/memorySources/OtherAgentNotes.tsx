import React from 'react';
import { memorySourcesApi } from '../../lib/endpoints';
import { WORDS, dayOf, fileCountLine, sizeLine, type Source } from '../../lib/memorySources.mts';
import { Section } from '../settings/kit';
import { useLoad } from './useLoad';

const itemCls = (on: boolean) =>
  `w-full text-left rounded-lg px-3 py-2 text-xs border ${
    on ? 'border-blue-500/40 bg-blue-600/10' : 'border-transparent hover:bg-slate-100 dark:hover:bg-white/5'
  }`;

/** What another agent keeps: its folders, their files, a file's text. Shown, never changed — nothing here calls a write. */
export const OtherAgentNotes: React.FC<{ source: Source }> = ({ source }) => {
  const topics = useLoad(() => memorySourcesApi.topics(source.id).then((r) => r.topics), [source.id]);
  const [topic, setTopic] = React.useState<string | null>(null);
  const [file, setFile] = React.useState<string | null>(null);
  React.useEffect(() => { setTopic(null); setFile(null); }, [source.id]);
  const files = useLoad(
    () => (topic ? memorySourcesApi.files(source.id, topic).then((r) => r.entries) : Promise.resolve(null)),
    [source.id, topic],
  );
  const text = useLoad(
    () => (file ? memorySourcesApi.fileText(source.id, file) : Promise.resolve(null)),
    [source.id, file],
  );

  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-[16rem_1fr]" data-testid="memory-readonly" data-source={source.id}>
      <Section title={WORDS.topics}>
        {topics.value?.length === 0 && <p className="text-[11px] text-slate-400">{WORDS.nothing}</p>}
        <ul className="space-y-0.5">
          {topics.value?.map((t) => (
            <li key={t.path}>
              <button className={itemCls(topic === t.path)} data-testid="memory-topic" data-path={t.path} title={t.path}
                onClick={() => { setTopic(t.path); setFile(null); }}>
                <span className="break-all" data-no-i18n>{t.name}</span>
                <span className="block text-[10px] text-slate-400">{fileCountLine(t.count)}</span>
              </button>
            </li>
          ))}
        </ul>
      </Section>
      <Section title={topic ? WORDS.fileAt : WORDS.topics} desc={WORDS.readOnly}>
        {topic && files.value?.length === 0 && <p className="text-xs text-slate-500">{WORDS.noEntries}</p>}
        <ul className="space-y-0.5">
          {files.value?.map((f) => (
            <li key={f.path}>
              <button className={itemCls(file === f.path)} data-testid="memory-file" data-path={f.path} onClick={() => setFile(f.path)}>
                <span data-no-i18n>{f.name}</span>
                <span className="block text-[10px] text-slate-400" data-no-i18n>{sizeLine(f.bytes)} · {dayOf(f.changed)}</span>
              </button>
            </li>
          ))}
        </ul>
        {text.value && (
          <div className="mt-4" data-testid="memory-file-text">
            <p className="break-all text-[11px] text-slate-500" data-testid="memory-file-path" data-no-i18n>{text.value.path}</p>
            <pre className="mt-2 max-h-[28rem] overflow-auto whitespace-pre-wrap rounded-lg bg-slate-100 p-3 text-[11px] dark:bg-white/5" data-no-i18n>{text.value.text}</pre>
            {text.value.truncated && <p className="mt-1 text-[11px] text-slate-400">{WORDS.truncated}</p>}
          </div>
        )}
      </Section>
    </div>
  );
};
