#!/usr/bin/env node
'use strict';

// node bin/send.js a@x.com b@x.com ...  -> prints one line per invitation.
const { build } = require('../lib/invites');

for (const mail of build(process.argv.slice(2))) {
  console.log(mail.to + ': ' + mail.subject);
}
