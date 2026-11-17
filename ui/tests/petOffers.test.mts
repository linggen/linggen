import { test } from 'node:test';
import assert from 'node:assert/strict';
import { needsConfirm, offerRequest, outcomeOf, stripOfferMarkers } from '../src/lib/petOffers.mts';

const scan = { label: 'Look what is big', kind: 'tool', skill: 'apple-shifu', tool: 'ScanDisk', args: null, tier: 'read', view: null };
const clear = { ...scan, label: 'Clear it', tool: 'Clear', tier: 'edit', args: { what: 'caches' } };
const files = { label: 'Open Files', kind: 'open', skill: 'apple-shifu', tool: null, args: null, tier: null, view: 'files' };

test('the button posts the named tool through the skill-tool route', () => {
  assert.deepEqual(offerRequest(scan), { url: '/api/skills/apple-shifu/tools/ScanDisk', body: {} });
  assert.deepEqual(offerRequest(clear).body, { what: 'caches' });
});

test('a place opens through the app launch route, at its view', () => {
  assert.deepEqual(offerRequest(files), { url: '/api/apps/launch', body: { skill: 'apple-shifu', view: 'files' } });
});

test('a tool that changes anything asks first; a read and a place do not', () => {
  assert.equal(needsConfirm(clear), true);
  assert.equal(needsConfirm(scan), false);
  assert.equal(needsConfirm(files), false);
});

test('the outcome is the tool\'s own exit code', () => {
  assert.equal(outcomeOf(scan, true, { exit_code: 0 }), 'done');
  assert.equal(outcomeOf(scan, true, { exit_code: 2 }), 'failed');
  assert.equal(outcomeOf(scan, false, null), 'failed');
  assert.equal(outcomeOf(files, true, { ok: true }), 'done');
});

test('her thread row keeps the line and the labels, never the markers', () => {
  const r = stripOfferMarkers('Your Mac is nearly full. [offer 2: Look what is big] [offer 1: Open Files]');
  assert.equal(r.text, 'Your Mac is nearly full.');
  assert.deepEqual(r.labels, ['Look what is big', 'Open Files']);
  assert.deepEqual(stripOfferMarkers('Just a line.'), { text: 'Just a line.', labels: [] });
});
