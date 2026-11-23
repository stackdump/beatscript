// What the page sends when it stores a program (the short link's PUT /api/shares, streaming and export's PUT /api/programs):
// its canonical text (what the CID is of) with its provenance lines after it, and no other comment. A provenance line says
// where written-out notes came from and the CID that pins them (docs/beatscript/ext-ffi.md, "Saved"): eject's `# from: <call> -> <cid>`,
// or expand's `# from <cid>.<section>` over a section it borrowed. The server keeps them as meta.based_on; comments are not in
// the CID, so the CID stays the canonical text's. Doc comments, `#!` header lines and every other comment stay on the device (a BeatScript 2
// text's version line is not a comment: it is the canonical text's first line, in the CID, so it is sent first).
//
// The rule is djlive refs::pinned's (the server applies it again to what it is sent), read from the comments `fmt` answers
// (`comments`: each comment's text, its `#` marks stripped and trimmed).

const CID = /^z[1-9A-HJ-NP-Za-km-z]{20,80}$/;
const WORD = /^[A-Za-z0-9_-]+$/;

/** One comment's text (as fmt's `comments` give it) as a provenance line `<call> -> <cid>`, or null. */
export function provenanceLine(note) {
  if (typeof note !== 'string') return null;
  const n = note.trim();
  let call, cid;
  if (n.startsWith('from:')) {
    const rest = n.slice(5).trim();
    const i = rest.lastIndexOf(' -> ');
    if (i < 0) return null;
    call = rest.slice(0, i).trim();
    cid = rest.slice(i + 4).trim();
  } else if (n.startsWith('from ')) {
    const r = n.slice(5).trim();
    const dot = r.indexOf('.');
    if (dot < 0 || !r.slice(dot + 1).split('.').every((w) => WORD.test(w))) return null;
    call = r;
    cid = r.slice(0, dot);
  } else {
    return null;
  }
  if (!call || !CID.test(cid)) return null;
  // a call that names a stored program is pinned by that program
  const lead = call.split('.')[0];
  if (CID.test(lead) && lead !== cid) return null;
  return `${call} -> ${cid}`;
}

/** The provenance lines among a text's comments, in order, each once. */
export function provenanceLines(comments) {
  const out = [];
  for (const c of Array.isArray(comments) ? comments : []) {
    const l = provenanceLine(c);
    if (l && !out.includes(l)) out.push(l);
  }
  return out;
}

/** The text to store: the canonical text, then a `# from: <call> -> <cid>` line for each provenance line (none: the canonical text). */
export function storedText(canonical, comments) {
  const lines = provenanceLines(comments);
  if (!lines.length) return canonical;
  const head = canonical.endsWith('\n') ? canonical : `${canonical}\n`;
  return head + lines.map((l) => `# from: ${l}\n`).join('');
}
