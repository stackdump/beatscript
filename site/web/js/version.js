// The language version a text says it is in (crates/djlive/src/version.rs, in JS for the page and its mock): the version
// line in the `#!` header block. `#! beatscript 2` is BeatScript 2; `#! btw 1`, or no version line, is BeatScript 1, the
// spelling before the rename, which keeps its old canonical text and CID. New text the page writes says LINE first.

export const LINE = '#! beatscript 2';
export const LEGACY_LINE = '#! btw 1';
export const CANONICAL_HEAD = `${LINE}\n`;

/** The header block's version line: {version, start, end} (end before its newline), null for none, or {error} for one that does not read. */
export function findVersionLine(src) {
  let found = null;
  let at = 0;
  for (const raw of String(src).split(/(?<=\n)/)) {
    const start = at;
    at += raw.length;
    const t = raw.trimEnd();
    if (!t.startsWith('#!')) break;
    const rest = t.slice(2).trim();
    if (rest.includes(':')) continue;
    const [name, ...more] = rest.split(/\s+/);
    if (name !== 'btw' && name !== 'beatscript') continue;
    const number = more.join(' ');
    let version;
    if (name === 'beatscript' && number === '2') version = 2;
    else if (name === 'btw' && number === '1') version = 1;
    else return { error: `\`${rest}\` is not a version of the language`, start, end: start + t.length };
    if (found) return { error: 'a text has one version line', start, end: start + t.length };
    found = { version, start, end: start + t.length };
  }
  return found;
}

/** 2 for a text that says `#! beatscript 2`, else 1. */
export function versionOf(src) {
  const l = findVersionLine(src);
  return l && !l.error ? l.version : 1;
}

/** `src` in BeatScript 2 spelling: its `#! btw 1` line made LINE, or LINE put first; already 2 (or a bad version line): as it is. */
export function upgradeSource(src) {
  const s = String(src);
  const l = findVersionLine(s);
  if (l && (l.error || l.version === 2)) return s;
  if (l) return `${s.slice(0, l.start)}${LINE}${s.slice(l.end)}`;
  return `${LINE}\n${s}`;
}
