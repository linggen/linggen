/** The launcher's name for an app: what its SKILL.md `app.title` declares,
 *  else the skill's own name. The engine names no app (skill-spec § Apps). */
export const appLabel = (s: { name: string; app?: { title?: string } }): string =>
  s.app?.title?.trim() || s.name;
