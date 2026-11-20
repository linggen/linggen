import { appLabel } from '../../apps/appLabel.mts';
import { APPS_GROUP } from '../../lib/settingsSections.mts';
import type { SkillInfoFull } from '../../types.ts';

export function appSettingsEntries(skills: readonly SkillInfoFull[]) {
  return skills.flatMap(({ name, app }) => {
    if (app?.launcher !== 'web' || !app.settings) return [];
    const segments = app.settings.split('/');
    if (!name || name === '.' || name === '..' || /[/\\]/.test(name)
      || segments.some((part) => !part || part.includes('..') || part === '.' || part.includes('\\'))) return [];
    // Each component is its own URL segment, not URL syntax supplied by a skill.
    const path = segments.map(encodeURIComponent).join('/');
    return [{ id: `app-${name}` as const, label: appLabel({ name, app }), group: APPS_GROUP,
      src: `/apps/${encodeURIComponent(name)}/${path}?embed=1` }];
  });
}
