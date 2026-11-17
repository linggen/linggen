'use strict';

const { dedupeFast } = require('./dedupeFast');

// One invitation per person: [{ to, subject }].
function build(emails) {
  const valid = emails.filter(e => typeof e === 'string' && e.includes('@'));
  return dedupeFast(valid).map(to => ({ to, subject: 'You are invited' }));
}

module.exports = { build };
