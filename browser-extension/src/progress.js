// Progress for the op in flight. The daemon shows the latest line on the
// running tool, so a wait on the user (the approval popup) or on another
// browser action reads as a wait instead of a hang. Control ops run one at a
// time, so a single current id is exact for anything the gate reports.

let sink = null;
let currentId = null;

export function setProgressSink(fn) {
  sink = fn;
}

export function beginOp(id) {
  currentId = id;
}

export function endOp(id) {
  if (currentId === id) currentId = null;
}

/// A line for [id] — used before the op starts, while it waits its turn.
export function progressFor(id, stage, text) {
  if (sink && id) sink({ t: "progress", id, stage, text });
}

/// A line for whichever op is running now.
export function progress(stage, text) {
  progressFor(currentId, stage, text);
}
