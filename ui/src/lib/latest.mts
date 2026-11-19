/**
 * The newest request wins. A list, a search or a file read answers late: by
 * then the person may have picked another file or typed another query, and
 * an older answer written over the newer one shows (or saves) the wrong
 * thing. Each request takes a ticket when it starts; its answer is applied
 * only while no newer request has started. (A request bound to a session
 * checks `stillOn` in sessionLoad.mts instead.) Pure, so node tests can
 * hold it.
 */

/** A source of tickets: `begin()` starts a request and returns its `current()`. */
export function latestOnly(): { begin: () => () => boolean } {
  let newest = 0;
  return {
    begin() {
      const mine = ++newest;
      return () => mine === newest;
    },
  };
}
