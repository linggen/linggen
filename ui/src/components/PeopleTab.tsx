import React from 'react';
import { User } from 'lucide-react';
import { ApiError, apiErrorMessage } from '../lib/api';
import { peopleApi } from '../lib/endpoints';
import {
  LEVELS, WORDS, appChoices, appRows, personTitle, splitApps, appsAfterToggle, arrivalLine, deviceLabel, limitFromText, limitText,
  modelRows, modelsAfterToggle, offeredApps, usedLine,
  type AppChoice, type AppRow, type PeopleReply, type Person, type PersonPatch,
} from '../lib/people.mts';
import { useServerStore } from '../stores/serverStore';
import { Section, inputCls, sectionCls } from './settings/kit';

const POLL_MS = 5000;
const OWNER_KEY = 'owner';

const chipCls = (on: boolean, locked = false) =>
  `px-2 py-1 rounded-md text-[11px] font-medium border transition-colors ${
    on
      ? 'bg-blue-600/10 border-blue-500/40 text-blue-600 dark:text-blue-400'
      : 'border-slate-200 dark:border-white/10 text-slate-500 hover:border-slate-300 dark:hover:border-white/20'
  } ${locked ? 'opacity-60 cursor-not-allowed' : ''}`;
const labelCls = 'text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400';
const hintCls = 'text-[11px] text-slate-400 mt-1';

/** The household as the engine answers it, kept current; `save` sends one change and shows what the engine then says. */
function usePeople() {
  const [reply, setReply] = React.useState<PeopleReply | null>(null);
  const [busy, setBusy] = React.useState<ReadonlySet<string>>(new Set());
  const [refused, setRefused] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  // Replies can land out of order (the poll, a save's refresh): only the latest ask is shown.
  const asked = React.useRef(0);
  const refresh = React.useCallback(async () => {
    const mine = ++asked.current;
    try {
      const next = await peopleApi.list();
      if (mine !== asked.current) return;
      setReply(next);
      setRefused(false);
    } catch (e) {
      // Below admin the engine refuses the list: the page is not at this Mac.
      if (e instanceof ApiError && e.status === 403) setRefused(true);
    }
  }, []);

  React.useEffect(() => {
    refresh();
    const t = setInterval(refresh, POLL_MS);
    return () => clearInterval(t);
  }, [refresh]);

  const save = React.useCallback(async (key: string, change: () => Promise<unknown>) => {
    setBusy((now) => new Set(now).add(key));
    setError(null);
    try {
      await change();
    } catch (e) {
      if (e instanceof ApiError && e.status === 403) setRefused(true);
      else setError(apiErrorMessage(e));
    } finally {
      await refresh();
      setBusy((now) => {
        const next = new Set(now);
        next.delete(key);
        return next;
      });
    }
  }, [refresh]);

  return { reply, busy, refused, error, save };
}

export const PeopleTab: React.FC = () => {
  const { reply, busy, refused, error, save } = usePeople();
  const models = useServerStore((s) => s.models);
  const macModels = React.useMemo(() => models.map((m) => m.id).filter(Boolean), [models]);

  if (refused || (reply && !reply.at_this_mac)) {
    return <Section title={WORDS.title}><p className="text-xs text-slate-500 dark:text-slate-400">{WORDS.notHere}</p></Section>;
  }
  if (!reply) {
    return <Section title={WORDS.title}><p className="text-xs text-slate-400">{WORDS.loading}</p></Section>;
  }

  const mineOnly = reply.owner.mine_only;
  const apps = appChoices(reply.apps);
  const setPerson = (person: Person, patch: PersonPatch) => save(person.id, () => peopleApi.set(person.id, patch));
  return (
    <div className="space-y-6">
      <Section title={WORDS.title} desc={WORDS.intro}>
        {error && <p role="alert" className="text-xs text-red-500">{error}</p>}
        {reply.people.length === 0 && <p className="text-xs text-slate-500 dark:text-slate-400">{WORDS.none}</p>}
      </Section>
      {reply.people.map((person) => (
        <PersonCard
          key={person.id}
          person={person}
          title={personTitle(person, reply.people)}
          apps={apps}
          mineOnly={mineOnly}
          macModels={macModels}
          busy={busy.has(person.id)}
          onChange={(patch) => setPerson(person, patch)}
        />
      ))}
      <OwnerSide apps={apps} mineOnly={mineOnly} busy={busy.has(OWNER_KEY)}
        onChange={(next) => save(OWNER_KEY, () => peopleApi.keepForOwner(next))} />
    </div>
  );
};

const PersonCard: React.FC<{
  person: Person;
  title: string;
  apps: AppChoice[];
  mineOnly: string[];
  macModels: string[];
  busy: boolean;
  onChange: (patch: PersonPatch) => void;
}> = ({ person, title, apps, mineOnly, macModels, busy, onChange }) => {
  const { settings } = person;
  const offered = offeredApps(apps, mineOnly);
  const arrival = arrivalLine(settings.arrival);
  return (
    <section className={sectionCls} data-testid="person" data-person={person.id}>
      <div className="flex items-center gap-2">
        <User size={16} className="text-slate-400 shrink-0" />
        <h3 className="text-sm font-semibold text-slate-800 dark:text-slate-200">{title}</h3>
      </div>
      <ul className="mt-1 ml-6 text-[11px] text-slate-400">
        {person.devices.map((d) => <li key={d.id}>{deviceLabel(d)}</li>)}
      </ul>
      {arrival && (
        <p className="mt-3 flex items-center gap-2 text-xs text-amber-700 dark:text-amber-400" data-testid="arrival">
          <span>{arrival}</span>
          <button className="underline" onClick={() => onChange({ arrival: null })}>{WORDS.dismiss}</button>
        </p>
      )}
      <fieldset disabled={busy} className="mt-4 space-y-5 min-w-0 disabled:opacity-70">
        <div>
          <span className={labelCls}>{WORDS.level}</span>
          <div className="mt-1 inline-flex rounded-md border border-slate-200 dark:border-white/10 overflow-hidden" role="group" aria-label={WORDS.level}>
            {LEVELS.map(({ id, label }) => (
              <button
                key={id}
                aria-pressed={person.level === id}
                onClick={() => onChange({ level: id })}
                className={`px-3 py-1 text-[11px] font-medium transition-colors ${
                  person.level === id ? 'bg-blue-600 text-white' : 'text-slate-500 hover:bg-slate-100 dark:hover:bg-white/10'
                }`}
              >
                {label}
              </button>
            ))}
          </div>
        </div>

        <div>
          <span className={labelCls}>{WORDS.apps}</span>
          <GroupedApps
            rows={appRows(apps, mineOnly, settings.apps)}
            lead={(
              <button
                aria-pressed={settings.apps == null}
                onClick={() => onChange({ apps: null })}
                className={chipCls(settings.apps == null)}
              >
                {WORDS.allApps}
              </button>
            )}
            chip={(row) => (
              <button
                key={row.name}
                data-app={row.name}
                aria-pressed={row.on}
                disabled={row.kept}
                onClick={() => onChange({ apps: appsAfterToggle(settings.apps, offered, row.name) })}
                className={chipCls(row.on, row.kept)}
              >
                {row.on ? '✓ ' : ''}{row.label}
                {row.kept && <span className="ml-1 text-slate-400">{WORDS.keptForYou}</span>}
                {row.gone && <span className="ml-1 text-slate-400">{WORDS.gone}</span>}
              </button>
            )}
          />
          {apps.length === 0 && <p className={hintCls}>{WORDS.noApps}</p>}
        </div>

        <div>
          <span className={labelCls}>{WORDS.models}</span>
          <p className={hintCls}>{WORDS.modelsHint}</p>
          <div className="mt-1.5 flex flex-wrap gap-1.5">
            {modelRows(macModels, settings.models).map((row) => (
              <button
                key={row.id}
                data-model={row.id}
                aria-pressed={row.on}
                onClick={() => onChange({ models: modelsAfterToggle(settings.models, row.id) })}
                className={chipCls(row.on)}
              >
                {row.on ? '✓ ' : ''}{row.id}
                {row.gone && <span className="ml-1 text-slate-400">{WORDS.gone}</span>}
              </button>
            ))}
          </div>
          {settings.models.length === 0 && <p className="text-[11px] text-slate-500 dark:text-slate-400 mt-1.5">{WORDS.noModel}</p>}
        </div>

        <LimitField
          limit={settings.daily_tokens}
          used={person.used_today}
          onCommit={(limit) => onChange({ daily_tokens: limit })}
        />
      </fieldset>
    </section>
  );
};

/** The daily limit: typed, and sent when the field is left or Enter is pressed. */
const LimitField: React.FC<{
  limit: number | null | undefined;
  used: number;
  onCommit: (limit: number | null) => void;
}> = ({ limit, used, onCommit }) => {
  const [text, setText] = React.useState(limitText(limit));
  const [bad, setBad] = React.useState(false);
  // What the engine says wins whenever it changes (a save, another change).
  React.useEffect(() => {
    setText(limitText(limit));
    setBad(false);
  }, [limit]);

  const commit = () => {
    const read = limitFromText(text);
    if ('bad' in read) return setBad(true);
    setBad(false);
    if (read.limit !== (limit ?? null)) onCommit(read.limit);
  };
  return (
    <div>
      <span className={labelCls}>{WORDS.limit}</span>
      <div className="mt-1 flex items-center gap-2">
        <input
          aria-label={WORDS.limit}
          inputMode="numeric"
          className={`${inputCls} !w-40`}
          value={text}
          placeholder={WORDS.limitPlaceholder}
          onChange={(e) => setText(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing || e.keyCode === 229) return;
            if (e.key === 'Enter') commit();
          }}
        />
        <span className="text-[11px] text-slate-500 dark:text-slate-400">{WORDS.tokensADay}</span>
      </div>
      {bad && <p className="text-[11px] text-red-500 mt-1">{WORDS.badNumber}</p>}
      <p className={hintCls} data-testid="used-today">{usedLine(used, limit)}</p>
    </div>
  );
};

/** The apps no one else uses — one list, for everyone. */
const OwnerSide: React.FC<{
  apps: AppChoice[];
  mineOnly: string[];
  busy: boolean;
  onChange: (next: string[]) => void;
}> = ({ apps, mineOnly, busy, onChange }) => {
  const rows = appRows(apps, [], mineOnly);
  return (
    <Section title={WORDS.mineOnly} desc={WORDS.mineOnlyHint}>
      <div data-testid="mine-only">
        <GroupedApps
          rows={rows}
          chip={(row) => (
            <button
              key={row.name}
              data-app={row.name}
              aria-pressed={row.on}
              disabled={busy}
              onClick={() => onChange(row.on ? mineOnly.filter((n) => n !== row.name) : [...mineOnly, row.name])}
              className={chipCls(row.on)}
            >
              {row.on ? '✓ ' : ''}{row.label}
              {row.gone && <span className="ml-1 text-slate-400">{WORDS.gone}</span>}
            </button>
          )}
        />
      </div>
      {apps.length === 0 && mineOnly.length === 0 && <p className={hintCls}>{WORDS.noApps}</p>}
    </Section>
  );
};

/** A checklist of skills: the ones with an app page first, the others under their own label. */
const GroupedApps: React.FC<{
  rows: AppRow[];
  lead?: React.ReactNode;
  chip: (row: AppRow) => React.ReactNode;
}> = ({ rows, lead, chip }) => {
  const { apps, others } = splitApps(rows);
  return (
    <>
      <div className="mt-1 flex flex-wrap gap-1.5">{lead}{apps.map(chip)}</div>
      {others.length > 0 && (
        <>
          <span className={`${labelCls} block mt-3`}>{WORDS.otherSkills}</span>
          <div className="mt-1 flex flex-wrap gap-1.5">{others.map(chip)}</div>
        </>
      )}
    </>
  );
};
