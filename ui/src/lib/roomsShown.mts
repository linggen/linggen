// Rooms show on a page only while the engine says rooms are on (page state's
// `rooms_on`, the engine's one switch). Until it says so — a page just
// opened, an engine that never set it — nothing about rooms shows.

/** Whether anything about rooms shows. `on` is the engine's word; anything
 *  but `true` reads as off. */
export function roomsShown(on: unknown): boolean {
  return on === true;
}
