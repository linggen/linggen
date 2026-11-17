'use strict';

// For the nightly list (50k addresses and more): one Set, one pass.
// Addresses compare case-insensitively; the first spelling seen is kept.
function dedupeFast(list) {
  const seen = new Set();
  const out = [];
  for (const address of list) {
    const key = address.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(address);
  }
  return out;
}

module.exports = { dedupeFast };
