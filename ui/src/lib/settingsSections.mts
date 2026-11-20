import { roomsShown } from './roomsShown.mts';

type Requirement = 'rooms' | 'companion' | 'shell';
type SectionDeclaration = {
  id: string;
  label: string;
  group: string | null;
  bare: boolean;
  needsConfig: boolean;
  requires: readonly Requirement[];
};

/** Sidebar order and availability. Labels are translated in strings-zh.ts. */
export const SETTINGS_SECTIONS = [
  { id: 'general', label: 'General', group: null, bare: true, needsConfig: true, requires: [] },
  { id: 'account', label: 'Account', group: null, bare: false, needsConfig: false, requires: [] },
  { id: 'desktop', label: 'Desktop', group: null, bare: false, needsConfig: false, requires: ['shell'] },
  { id: 'agents', label: 'Agents', group: null, bare: true, needsConfig: true, requires: [] },
  { id: 'models', label: 'Models', group: null, bare: true, needsConfig: true, requires: [] },
  { id: 'session', label: 'Session', group: null, bare: true, needsConfig: true, requires: [] },
  { id: 'skills', label: 'Skills', group: 'Capabilities', bare: true, needsConfig: false, requires: [] },
  { id: 'tools', label: 'Tools & MCP', group: 'Capabilities', bare: true, needsConfig: false, requires: [] },
  { id: 'mission', label: 'Missions', group: 'Capabilities', bare: false, needsConfig: false, requires: [] },
  { id: 'phone', label: 'Phone', group: 'Devices', bare: false, needsConfig: true, requires: [] },
  { id: 'room', label: 'Room', group: 'Devices', bare: true, needsConfig: false, requires: ['rooms'] },
  { id: 'memory', label: 'Memory', group: 'Data', bare: false, needsConfig: false, requires: [] },
  { id: 'storage', label: 'Storage', group: 'Data', bare: false, needsConfig: false, requires: [] },
] as const satisfies readonly SectionDeclaration[];

export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];
export type SettingsSectionId = SettingsSection['id'];

const ALIASES = { mcp: 'tools' } as const satisfies Record<string, SettingsSectionId>;
export type SettingsTab = SettingsSectionId | keyof typeof ALIASES;

export function resolveSettingsTab(tab: string | null | undefined) {
  return tab && Object.hasOwn(ALIASES, tab) ? ALIASES[tab as keyof typeof ALIASES] : tab;
}

export type SettingsRequirements = Record<Requirement, boolean>;

export function sectionAvailable(section: SettingsSection, requirements: SettingsRequirements): boolean {
  return section.requires.every((requirement) => requirement === 'rooms'
    ? roomsShown(requirements.rooms)
    : requirements[requirement]);
}

export const BARE_SECTIONS = SETTINGS_SECTIONS.filter((section) => section.bare);

export const APPS_GROUP = 'Apps';
// Step 2 replaces this list with a settings field declared by the skill.
const HAS_SETTINGS: ReadonlySet<string> = new Set(['cfo', 'apple-shifu', 'pulse', 'dj', 'lingjing']);
export const appHasSettings = (name: string) => HAS_SETTINGS.has(name);
