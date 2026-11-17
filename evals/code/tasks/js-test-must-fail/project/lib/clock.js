'use strict';

// Time is passed in as milliseconds so that tests never wait.
function now() {
  return Date.now();
}

module.exports = { now };
