'use strict';

const { dedupe } = require('./dedupe');

// The address list as a CSV column, for the newsletter tool.
function toCsvColumn(emails) {
  return ['email'].concat(dedupe(emails)).join('\n');
}

module.exports = { toCsvColumn };
