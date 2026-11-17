'use strict';

// For the nightly list (50k addresses and more): one Set, one pass.
function dedupeFast(list) {
  return Array.from(new Set(list));
}

module.exports = { dedupeFast };
