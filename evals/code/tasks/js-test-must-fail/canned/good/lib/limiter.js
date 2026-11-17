'use strict';

const clock = require('./clock');

// At most `limit` requests per `windowMs` for each key.
class Limiter {
  constructor(options) {
    this.limit = options.limit;
    this.windowMs = options.windowMs;
    this.hits = new Map();
  }

  // May this key make a request at time `at` (ms)? Records it when allowed.
  allow(key, at) {
    const t = at === undefined ? clock.now() : at;
    const recent = (this.hits.get(key) || []).filter(h => h > t - this.windowMs);
    if (recent.length >= this.limit) {
      this.hits.set(key, recent);
      return false;
    }
    recent.push(t);
    this.hits.set(key, recent);
    return true;
  }

  // How many requests this key may still make right now.
  remaining(key, at) {
    const t = at === undefined ? clock.now() : at;
    const recent = (this.hits.get(key) || []).filter(h => h > t - this.windowMs);
    return Math.max(0, this.limit - recent.length);
  }
}

module.exports = { Limiter };
