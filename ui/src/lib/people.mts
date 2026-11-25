/**
 * Settings → People: the household's people as the engine answers them
 * (`GET /api/people`, `server/api/people.rs`) and the few rules the page keeps
 * about them. The engine decides what a setting means and what holds; the page
 * shows its answer and sends what the owner changed — never a guess of its own.
 *
 * Every sentence the section shows is in `WORDS`, at most twenty words each
 * (tests/people.test.mts holds that, and a Chinese entry for each).
 */

import { appLabel, appListed } from '../apps/appLabel.mts';

export type Level = 'chat' | 'read' | 'edit';

/** The three levels an owner may give another person. Admin is never offered. */
export const LEVELS: readonly { id: Level; label: string }[] = [
  { id: 'chat', label: 'Chat' },
  { id: 'read', label: 'Read' },
  { id: 'edit', label: 'Edit' },
];

/** How a person's settings came to be their account's when their phone signed in. */
export type Arrival = 'moved' | 'kept';

export interface PersonSettings {
  level?: Level;
  /** Absent or null: all (but the apps the owner keeps). */
  apps?: string[] | null;
  models: string[];
  daily_tokens?: number | null;
  arrival?: Arrival;
}

export interface PersonDevice {
  id: string;
  name: string;
  kind: 'phone' | 'machine' | string;
}

export interface Person {
  /** The string the engine keeps the person as (an account id, or `device:<id>`). */
  id: string;
  name: string;
  settings: PersonSettings;
  /** The level the person stands at now — the engine's word, not `settings.level`. */
  level: Level;
  used_today: number;
  devices: PersonDevice[];
}

export interface PeopleReply {
  owner: { mine_only: string[] };
  people: Person[];
  /** Every installed skill a person's apps list may hold — the engine's own list. */
  apps: InstalledSkill[];
  /** The one who asks is at this Mac itself — only there are people set. */
  at_this_mac: boolean;
}

/** A skill the engine says a person's list may hold, and whether it has an app page. */
export interface InstalledSkill {
  name: string;
  app: boolean;
}

/** A skill as the lists offer it. */
export interface AppChoice extends InstalledSkill {
  /** The name the sidebar shows (`appLabel`), not the folder's. */
  label: string;
  /** The sidebar lists it (`appListed`); an unlisted app is held and kept, never drawn. */
  listed: boolean;
}

/** What the page knows of a skill beyond the engine's People list: the app block its SKILL.md declares. */
export interface SkillShown {
  name: string;
  app?: { title?: string; list?: boolean } | null;
}

/** The engine's list as the page offers it, apps first and then the other skills,
 *  each with the sidebar's name and whether the sidebar lists it. */
export function appChoices(installed: readonly InstalledSkill[], skills: readonly SkillShown[] = []): AppChoice[] {
  const choices = installed.map((s) => {
    const skill = skills.find((k) => k.name === s.name) ?? { name: s.name };
    return { ...s, label: appLabel(skill), listed: appListed(skill) };
  });
  return [...choices.filter((s) => s.app), ...choices.filter((s) => !s.app)];
}

/** What the engine calls the person behind a phone with no account (`GUEST_LABEL`,
 *  state_fs/devices/mod.rs — tests/peoplePage.test.mts holds the two equal). The page
 *  says "Signed-out phone" for it; the engine's word is kept where it records who did what. */
export const ENGINE_GUEST_LABEL = 'Guest';

/** What a card is titled: the person's name; a signed-out phone's person, when
 *  there are several, with the device's own name so they can be told apart. */
export function personTitle(person: Person, all: readonly Person[]): string {
  const signedOut = (p: Person) => p.id.startsWith('device:');
  const name = signedOut(person) && person.name === ENGINE_GUEST_LABEL ? WORDS.signedOutPhone : person.name;
  if (!signedOut(person) || all.filter(signedOut).length < 2) return name;
  return `${name} · ${person.devices[0]?.name ?? ''}`.trimEnd();
}

/** What a PUT to a person may carry (`People::set`). */
export type PersonPatch = Partial<{
  level: Level;
  apps: string[] | null;
  models: string[];
  daily_tokens: number | null;
  arrival: null;
}>;

/** The apps the owner has not kept and the sidebar lists: what "all" becomes when one is switched off.
 *  An unlisted app is never drawn, so it is never part of a list made by the owner's clicks. */
export function offeredApps(installed: readonly AppChoice[], mineOnly: readonly string[]): AppChoice[] {
  return installed.filter((app) => app.listed && !mineOnly.includes(app.name));
}

/** One line of a person's apps list: an app, whether it is chosen, and why it is not a choice. */
export interface AppRow extends AppChoice {
  on: boolean;
  /** The owner keeps it: shown, never a choice. */
  kept: boolean;
  /** Held in the list but no longer installed: shown so it can be taken out. */
  gone: boolean;
}

/** The rows the page draws: those the sidebar lists, apps first, the other skills apart.
 *  An unlisted app stays in every list the engine holds; it is only not drawn. */
export function splitApps(rows: readonly AppRow[]): { apps: AppRow[]; others: AppRow[] } {
  const shown = rows.filter((r) => r.listed);
  return { apps: shown.filter((r) => r.app), others: shown.filter((r) => !r.app) };
}

/** The apps shown for one person: every installed one, then any the list holds that is not installed. */
export function appRows(installed: readonly AppChoice[], mineOnly: readonly string[], apps: readonly string[] | null | undefined): AppRow[] {
  const rows = installed.map((app) => {
    const kept = mineOnly.includes(app.name);
    return { ...app, kept, gone: false, on: !kept && (apps == null || apps.includes(app.name)) };
  });
  const known = new Set(installed.map((app) => app.name));
  const gone = (apps ?? []).filter((name) => !known.has(name));
  return [...rows, ...gone.map((name) => ({ name, label: name, listed: true, app: false, on: true, kept: false, gone: true }))];
}

/** The list a person's apps become when `name` is switched: from "all" it is every offered app but that one. */
export function appsAfterToggle(apps: readonly string[] | null | undefined, offered: readonly AppChoice[], name: string): string[] {
  const now = apps ?? offered.map((app) => app.name);
  return now.includes(name) ? now.filter((n) => n !== name) : [...now, name];
}

/** A model line: one of this Mac's models, whether it is given, and whether it is gone from this Mac. */
export interface ModelRow {
  id: string;
  on: boolean;
  gone: boolean;
}

/** This Mac's models, then any the person holds that this Mac no longer has. */
export function modelRows(macModels: readonly string[], given: readonly string[]): ModelRow[] {
  const known = new Set(macModels);
  return [
    ...macModels.map((id) => ({ id, on: given.includes(id), gone: false })),
    ...given.filter((id) => !known.has(id)).map((id) => ({ id, on: true, gone: true })),
  ];
}

export function modelsAfterToggle(given: readonly string[], id: string): string[] {
  return given.includes(id) ? given.filter((m) => m !== id) : [...given, id];
}

/** What a typed limit means: none (blank), a whole number, or not a number. */
export function limitFromText(text: string): { limit: number | null } | { bad: true } {
  const plain = text.replace(/[\s,]/g, '');
  if (plain === '') return { limit: null };
  if (!/^\d{1,15}$/.test(plain)) return { bad: true };
  return { limit: Number(plain) };
}

export function limitText(limit: number | null | undefined): string {
  return limit == null ? '' : String(limit);
}

const count = (n: number) => n.toLocaleString('en-US');

/** "Used today: 1,200 of 200,000" — or without the limit when there is none. */
export function usedLine(used: number, limit: number | null | undefined): string {
  return limit == null ? `Used today: ${count(used)}` : `Used today: ${count(used)} of ${count(limit)}`;
}

/** The one line a sign-in that moved or kept settings is told in. */
export function arrivalLine(arrival: Arrival | undefined): string | null {
  return arrival ? WORDS.arrival[arrival] : null;
}

/** The person's one-line summary of their devices: "Friend's phone · Phone". */
export function deviceLabel(device: PersonDevice): string {
  return `${device.name} · ${device.kind === 'machine' ? WORDS.computer : WORDS.phone}`;
}

/** Everything the section says, in one place. */
export const WORDS = {
  title: 'People',
  intro: 'Who else uses this Mac, and what each person may use. A change holds at once.',
  none: 'No one else yet. A person appears here once their phone is paired.',
  notHere: 'People are set at this Mac itself, not from here.',
  loading: 'Loading…',
  level: 'How much they may do',
  apps: 'Apps',
  allApps: 'All apps',
  keptForYou: 'kept for you',
  gone: 'not installed',
  more: 'More',
  signedOutPhone: 'Signed-out phone',
  models: 'Models of this Mac',
  modelsHint: 'Their turns here run only on these. Their phone still has its own.',
  noModel: 'No model of this Mac — they use their own',
  limit: 'Daily limit',
  limitPlaceholder: 'No limit',
  tokensADay: 'tokens a day',
  badNumber: 'A whole number of tokens, or blank for no limit.',
  phone: 'Phone',
  computer: 'Computer',
  dismiss: 'Got it',
  mineOnly: 'Kept for you',
  mineOnlyHint: 'Apps no one else uses. Others never see them.',
  noApps: 'No apps installed.',
  arrival: {
    moved: 'Signed in: what you chose for this phone is now its account’s.',
    kept: 'Signed in: the account already had settings, so those stand.',
  },
} as const;

/** Every sentence of `WORDS`, flat — what the tests walk. */
export function allWords(): string[] {
  const out: string[] = [];
  const walk = (value: unknown) => {
    if (typeof value === 'string') out.push(value);
    else if (value && typeof value === 'object') Object.values(value).forEach(walk);
  };
  walk(WORDS);
  return out;
}
