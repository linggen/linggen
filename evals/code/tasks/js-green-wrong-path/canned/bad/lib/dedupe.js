'use strict';

// Keep the first of each address. Addresses compare case-insensitively
// (and ignore stray spaces).
function dedupe(list) {
  const seen = new Set();
  const out = [];
  for (const address of list) {
    const key = address.trim().toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(address);
  }
  return out;
}

module.exports = { dedupe };
