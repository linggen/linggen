'use strict';

// Keep the first of each address. Addresses compare case-insensitively.
function dedupe(list) {
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

module.exports = { dedupe };
