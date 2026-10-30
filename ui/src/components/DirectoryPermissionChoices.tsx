import type { AskUserQuestion } from '../types/generated/AskUserQuestion';

const modeTitle: Record<string, string> = {
  chat: 'Chat',
  read: 'Read',
  edit: 'Edit',
  admin: 'Admin',
};

type Grant =
  | { kind: 'dir'; label: string; mode: string; path: string }
  | { kind: 'all'; label: string; mode: string };

// `grant:at:<mode>:<path>` is one ancestor. `grant:all:<mode>` is everywhere.
function parseGrant(label: string): Grant | null {
  if (label.startsWith('grant:at:')) {
    const rest = label.slice('grant:at:'.length);
    const split = rest.indexOf(':');
    if (split <= 0) return null;
    const mode = rest.slice(0, split);
    const path = rest.slice(split + 1);
    if (!mode || !path) return null;
    return { kind: 'dir', label, mode, path };
  }
  if (label.startsWith('grant:all:')) {
    const mode = label.slice('grant:all:'.length);
    if (!mode || mode.includes(':')) return null;
    return { kind: 'all', label, mode };
  }
  return null;
}

// `~/repo`, `~/repo/a`, `~/repo/a/b` → `repo`, `repo/a`, `repo/a/b`.
function shortDir(path: string, broadest: string): string {
  const slash = broadest.lastIndexOf('/');
  const parent = slash >= 0 ? broadest.slice(0, slash) : '';
  if (parent && (path === broadest || path.startsWith(`${parent}/`))) {
    return path.slice(parent.length + 1);
  }
  return path;
}

export function DirectoryPermissionChoices({ question, onChoose, canGrant }: {
  question: AskUserQuestion;
  onChoose: (label: string) => void;
  canGrant: boolean;
}) {
  const grants = canGrant
    ? question.options.flatMap((option) => {
        const grant = parseGrant(option.label);
        return grant ? [grant] : [];
      })
    : [];
  const dirs = grants.flatMap((grant) => grant.kind === 'dir' ? [grant] : []);
  const everywhere = grants.filter((grant) => grant.kind === 'all');
  const rest = question.options.filter((option) => !parseGrant(option.label));
  const broadest = dirs[0]?.path ?? '';
  const target = question.permission?.path ?? '';

  return (
    <div className="flex flex-col gap-1.5 px-3 py-3">
      {target && (
        <div className="break-all font-mono text-[12px] text-slate-600 dark:text-slate-300" data-no-i18n data-testid="permission-path">
          {target}
        </div>
      )}
      {dirs.map((grant) => {
        const title = modeTitle[grant.mode] ?? grant.mode;
        const short = shortDir(grant.path, broadest);
        return (
          <button
            key={grant.label}
            type="button"
            data-testid="permission-directory"
            onClick={() => onChoose(grant.label)}
            className="rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-left text-[13px] font-semibold text-amber-900 hover:bg-amber-100 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200 dark:hover:bg-amber-500/20"
          >
            {`${title} · ${short}`}
          </button>
        );
      })}
      {everywhere.map((grant) => {
        const title = modeTitle[grant.mode] ?? grant.mode;
        return (
          <button
            key={grant.label}
            type="button"
            onClick={() => onChoose(grant.label)}
            className="rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-left text-[13px] font-semibold text-amber-900 hover:bg-amber-100 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200 dark:hover:bg-amber-500/20"
          >
            {`${title} everywhere`}
          </button>
        );
      })}
      {rest.map((option) => (
        <button
          key={option.label}
          type="button"
          onClick={() => onChoose(option.label)}
          className="rounded-lg border border-slate-200 bg-white px-3 py-2 text-left text-[13px] font-semibold text-slate-500 hover:border-red-300 hover:text-red-500 dark:border-white/10 dark:bg-white/5 dark:text-slate-400 dark:hover:border-red-500/40 dark:hover:text-red-400"
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
