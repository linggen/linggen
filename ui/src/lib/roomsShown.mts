// Rooms show on a page only while the engine says rooms are on (page state's
// `rooms_on`, the engine's one switch). Until it says so — a page just
// opened, an engine that never set it — nothing about rooms shows.

/** Whether anything about rooms shows. `on` is the engine's word; anything
 *  but `true` reads as off. */
export function roomsShown(on: unknown): boolean {
  return on === true;
}

/** Settings tab groups without the room tab unless rooms show. */
export function tabGroupsFor<T extends { key: string }, G extends { tabs: T[] }>(
  groups: G[],
  on: unknown,
): G[] {
  if (roomsShown(on)) return groups;
  return groups
    .map((g) => ({ ...g, tabs: g.tabs.filter((t) => t.key !== 'room') }))
    .filter((g) => g.tabs.length > 0);
}

/** Whether the settings section `key` (a tab, or a bare route) opens. */
export function sectionOpens(key: string | null | undefined, on: unknown): boolean {
  return key !== 'room' || roomsShown(on);
}
