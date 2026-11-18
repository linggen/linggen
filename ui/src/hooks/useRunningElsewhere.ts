/**
 * In a day's session: the display name of the place the running turn
 * `runId` stands in, when the engine told one and it is not this page's
 * (lib/dayFold.mts `runningElsewhere`); else null, as before. The status
 * line and Stop name it.
 */
import { useServerStore } from '../stores/serverStore';
import { useSessionStore } from '../stores/sessionStore';
import { pagePlace, placeName, runningElsewhere } from '../lib/dayFold.mts';

export function useRunningElsewhere(runId: string | null | undefined): string | null {
  const here = useSessionStore((s) => pagePlace(s.isSkillSession ? s.activeSkillName : null));
  const place = useServerStore((s) => runningElsewhere(runId, s.runPlaces, here));
  const skills = useServerStore((s) => s.skills);
  return place ? placeName(place, skills) : null;
}
