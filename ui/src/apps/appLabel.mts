/** The launcher's name for an app: what its SKILL.md `app.title` declares,
 *  else the skill's own name. The engine names no app (skill-spec § Apps). */
export const appLabel = (s: { name: string; app?: { title?: string } | null }): string =>
  s.app?.title?.trim() || s.name;

/** Whether the launcher lists an app: `list: false` in its SKILL.md keeps an
 *  unfinished app out of the sidebar's tab bar and out of the People page.
 *  The one rule both read; a skill with no app page is never hidden by it. */
export const appListed = (s: { app?: { list?: boolean } | null }): boolean => s.app?.list !== false;
