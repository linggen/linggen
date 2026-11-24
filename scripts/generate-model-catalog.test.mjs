import assert from 'node:assert/strict';
import { test } from 'node:test';
import { generate } from './generate-model-catalog.mjs';

const model = (id, name, family, day, status) => ({ id, name, family, release_date: day, status, modalities: {input:['text'], output:['text']} });
const google = (rows) => generate({google:{id:'google', name:'Google', models:Object.fromEntries(rows.map(m=>[m.id,m]))}}, '2026-11-23', ['google']).providers[0].models;

test('Gemini 4 siblings establish four tiers; only matching older names join', () => {
  const rows = ['Argon', 'Carbon', 'Hydrogen', 'Neon'].map(name => model(`gemini-4-${name.toLowerCase()}`, `Gemini 4 ${name}`, 'gemini', '2026-11-12'));
  rows.push(model('gemini-3-neon', 'Gemini 3 Neon', 'gemini', '2026-08-10'));
  rows.push(model('gemini-3', 'Gemini 3', 'gemini', '2026-08-10'));
  assert.deepEqual(google(rows).map(m=>m.family), ['gemini','gemini-neon','gemini-argon','gemini-carbon','gemini-hydrogen','gemini-neon']);
});

test('Gemini moving aliases are marked even when sharing the real release date', () => {
  const rows = [model('gemini-3.8-flash','Gemini 3.8 Flash','gemini-flash','2026-09-02'), model('gemini-flash-latest','Gemini Flash Latest','gemini-flash','2026-09-02'), model('gemini-flash-rolling','Gemini Flash Rolling','gemini-flash','2026-09-02','alias')];
  assert.deepEqual(google(rows).map(m=>m.stability), ['stable','alias','alias']);
});
