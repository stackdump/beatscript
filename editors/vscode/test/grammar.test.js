// The grammar's regexes compile (as JS, close enough to Oniguruma for the constructs used) and colour a sample as expected.
'use strict';
const assert = require('node:assert/strict');
const g = require('../syntaxes/beatscript.tmLanguage.json');

const rx = {};
for (const [k, v] of Object.entries(g.repository)) rx[k] = new RegExp(v.match || v.begin, 'g');

const line = 'deck A: kick e(5,8) vel=90..110 | bass $root inst=sub-bass   # a comment';
const hits = (k) => [...line.matchAll(rx[k])].map((m) => m[0]);
assert.deepEqual(hits('role'), ['kick', 'bass']);
assert.deepEqual(hits('call'), ['e']);
assert.deepEqual(hits('param').map((s) => s.replace('=', '')), ['vel', 'inst']);
assert.deepEqual(hits('dollar'), ['$root']);
assert.deepEqual(hits('comment'), ['# a comment']);
assert.deepEqual([...'section drop 16 bars {'.matchAll(rx.section)].map((m) => m[2]), ['drop']);
assert.ok(rx.header.test('#! genre: edm'));
// link 01's marks: a ghost `.` and a flam `&` glued to a step, a ratchet `:N`; `A.kick` keeps its dot uncoloured
const marks = (s) => [...s.matchAll(rx.operator)].map((m) => m[0]);
assert.deepEqual(marks('kick x. x& x:3 c3.^'), ['.', '&', ':', '.', '^']);
assert.deepEqual(marks('wire A.kick'), []);
assert.deepEqual(marks('vel=90..110'), ['=', '..']);
// an arrange mutation line begins at its mark, spaced (`~ acid rev`) or with a count glued to it (`~3 acid rev`, `@4 kik double`)
const mutation = new RegExp(g.repository.mutation.begin);
for (const l of ['  ~ acid rev', '  ~3 acid rev', '  @4 kik double']) assert.ok(mutation.test(l), l);
assert.ok(!mutation.test('  ~acid rev'));
console.log('grammar ok');
