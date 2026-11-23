// The ✦generate panel's pure parts (no DOM): the request the seeded generator takes, the `gen@2` call that names one
// part of what it prints, stepping a seed to its neighbour, and reading a generated program's structure and roles.
// Tested in site/test/generate-unit.mjs.

import { LENGTHS } from './station-model.js';

export { LENGTHS };

/** The roles a generated part line can name (btwapi::generate's parts; the MCP generate tool's `part` enum). */
export const ROLES = ['kick', 'snare', 'hihat', 'clap', 'bass', 'melody', 'harmony', 'arp'];

const U32 = 0x1_0000_0000;

/**
 * A seed as given -> the generator's 32-bit seed, as btwapi::generate::seed_value computes it: a whole number is itself
 * (its low 32 bits), a word its FNV-1a hash (trimmed, lower-cased, over its UTF-8 bytes), so "sunrise" is one track.
 */
export function seedValue(seed) {
  const s = String(seed == null ? '' : seed).trim();
  if (/^\d+$/.test(s)) return Number(BigInt(s) & 0xffffffffn);
  let h = 0x811c9dc5;
  for (const b of new TextEncoder().encode(s.toLowerCase())) {
    h ^= b;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}

/** The neighbouring seed, `d` away: a number steps (wrapping in 32 bits); a word steps from its hash, which is the same track. */
export function stepSeed(seed, d) {
  const v = seedValue(String(seed).trim() || '1');
  return String((((v + d) % U32) + U32) % U32);
}

/** A random seed, 0 to 99999 (short enough to say aloud). */
export function randomSeed(rand = Math.random) {
  return String(Math.floor(rand() * 100000));
}

/** The seed as a `gen@2` call can write it: a whole number or a NAME; any other word is written as its number (the same track). */
export function callSeed(seed) {
  const s = String(seed == null ? '' : seed).trim() || '1';
  if (/^\d+$/.test(s)) return String(seedValue(s));
  if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(s)) return s;
  return String(seedValue(s));
}

/** The request btw_generate / POST /api/generate take. */
export function genRequest({ genre, seed, length }) {
  return { genre: genre || 'techno', seed: String(seed == null ? '' : seed).trim() || '1', length: LENGTHS.includes(length) ? length : 'standard' };
}

/** `gen@2 techno seed=42 section=drop part=bass` (with ` length=L` unless it is standard): one part of one section, as a reference. */
export function genCall({ genre, seed, length, section, part }) {
  let s = `gen@2 ${genre || 'techno'} seed=${callSeed(seed)}`;
  if (length && length !== 'standard' && LENGTHS.includes(length)) s += ` length=${length}`;
  if (section) s += ` section=${section}`;
  if (part) s += ` part=${part}`;
  return s;
}

/** A check report -> {sections: [{name, bars}] in play order, bars: the song's bars}. */
export function structureOf(report) {
  const secs = (report && Array.isArray(report.sections) ? report.sections : []).map((x) => ({ name: String(x.name), bars: Number(x.bars) || 0 }));
  const bars = report && Number.isFinite(report.song_bars) ? report.song_bars : secs.reduce((a, x) => a + x.bars, 0);
  return { sections: secs, bars };
}

/** The roles a part line names: `deck A: kick pulse | snare backbeat` -> ['kick', 'snare']. */
function lineRoles(line) {
  const m = /^\s*deck\s+[AB]\s*:\s*(.*)$/.exec(line);
  if (!m) return [];
  return m[1].split('|').map((p) => p.trim().split(/\s+/)[0]).filter((r) => ROLES.includes(r));
}

/**
 * The roles that play in one section of a generated program, in ROLES order. A section's lines are part lines
 * (`deck A: ...`) and calls of the program's `def`s (`groove(pulse, hats)`), whose bodies are part lines.
 */
export function rolesIn(text, section) {
  const lines = String(text || '').split('\n');
  const defs = new Map();
  let cur = null; // {kind: 'def'|'section', name, depth}
  const found = new Set();
  for (const raw of lines) {
    const line = raw.replace(/#.*$/, '');
    if (!cur) {
      const d = /^def\s+([A-Za-z_][\w-]*)\s*\(/.exec(line);
      const s = /^section\s+([A-Za-z_][\w-]*)\b/.exec(line);
      if (d && line.includes('{')) { cur = { kind: 'def', name: d[1], depth: 1 }; defs.set(d[1], new Set()); continue; }
      if (s && line.includes('{')) { cur = { kind: 'section', name: s[1], depth: 1 }; continue; }
      continue;
    }
    const opens = (line.match(/\{/g) || []).length;
    const closes = (line.match(/\}/g) || []).length;
    if (cur.depth === 1) {
      if (cur.kind === 'def') for (const r of lineRoles(line)) defs.get(cur.name).add(r);
      else if (cur.name === section) {
        for (const r of lineRoles(line)) found.add(r);
        const call = /^\s*([A-Za-z_][\w-]*)\s*\(/.exec(line);
        if (call && defs.has(call[1])) for (const r of defs.get(call[1])) found.add(r);
      }
    }
    cur.depth += opens - closes;
    if (cur.depth <= 0) cur = null;
  }
  return ROLES.filter((r) => found.has(r));
}

/** The panel's remembered inputs (localStorage `btw.gen`), each checked: a bad value takes the default. */
export function loadGenState(store, genres) {
  let v = {};
  try { v = JSON.parse((store && store.getItem('btw.gen')) || '{}') || {}; } catch { v = {}; }
  return {
    genre: genres.includes(v.genre) ? v.genre : 'techno',
    seed: typeof v.seed === 'string' && v.seed.trim() && v.seed.length <= 64 ? v.seed.trim() : randomSeed(),
    length: LENGTHS.includes(v.length) ? v.length : 'standard',
    section: typeof v.section === 'string' ? v.section : '',
    part: ROLES.includes(v.part) ? v.part : '',
  };
}

export function saveGenState(store, st) {
  try { if (store) store.setItem('btw.gen', JSON.stringify({ genre: st.genre, seed: st.seed, length: st.length, section: st.section, part: st.part })); } catch { /* private mode */ }
}
