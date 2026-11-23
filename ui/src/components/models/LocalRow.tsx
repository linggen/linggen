/**
 * This Mac, as one row: the voice, hearing and picture lanes and any local
 * server. Facts only — what each is, its size and whether it is ready; a lane
 * is set up from its own place, not from here.
 */
import React, { useState } from 'react';
import { W, say, type Word } from '../../lib/modelsWords.mts';
import type { Kind, Provider } from '../../lib/providers.mts';
import { Row, Tag, hintCls } from './shell';

const KIND_WORD: Record<Kind, Word> = {
  chat: W.chat,
  live_voice: W.liveVoice,
  speak: W.readAloud,
  hear: W.dictation,
  draw: W.drawing,
};

/** What a lane that is not ready needs, in the engine's word for it. */
const needs = (fix: Provider['fix']): Word => (fix === 'download' ? W.notDownloaded : fix === 'install' ? W.notInstalled : W.notReady);

export const LocalRow: React.FC<{ providers: Provider[] }> = ({ providers }) => {
  const [open, setOpen] = useState(false);
  const lines = providers.flatMap((p) => p.models.map((m) => ({ p, m })));
  const kinds = [...new Set(lines.map(({ m }) => m.kind))];
  return (
    <Row
      testId="this-mac"
      name={W.thisMac}
      tag={<Tag>{W.kindLocal}</Tag>}
      subtitle={kinds.map((k) => KIND_WORD[k]).join(' · ')}
      open={open}
      onToggle={() => setOpen(!open)}
    >
      {lines.map(({ p, m }) => (
        <div key={`${p.id}${m.id}`} className="flex flex-wrap items-center gap-x-2 text-xs" data-testid="local-line">
          <Tag>{KIND_WORD[m.kind]}</Tag>
          <span className="min-w-0 break-all font-semibold" data-no-i18n>{m.model}</span>
          {p.disk_gb ? <span className={hintCls}>{say(W.gb, { n: p.disk_gb })}</span> : null}
          <span className={hintCls}>{m.ready ? W.ready : needs(p.fix)}</span>
        </div>
      ))}
    </Row>
  );
};
