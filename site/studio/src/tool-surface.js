// tool-surface.js: the studio's tools as an MCP server that lives in the page (the "in-artifact MCP").
//
// The local tools run in the wasm module (site/web/js/wasm.js makeFacade): no network, no sign-in. Their names and
// arguments are those of btwserve's MCP tools (crates/btwserve/src/mcp/tools.rs), less what only the server can do.
// The capability tools use what the host grants through window.claude.use(): compose and repair ask Claude
// (sample); store, render and queue call the viewer's BeatScript connector (mcp); the library keeps programs in the
// viewer's private part of the artifact's store (db and user); export offers a file (downloads); jam_send plays a
// program to everyone in the room (room). A tool whose capability is absent is still listed, with available false
// and a one-line why.
//
// call() answers MCP's CallToolResult and never rejects. handle() answers JSON-RPC 2.0 (initialize, tools/list,
// tools/call, ping). serve() puts handle() on postMessage for a parent frame (the config's bridge).
//
// The file imports nothing, so it works wherever it is copied: the WAV encoder comes in as encodeWav (the player's
// render-model.js wav) and the renderer as render.

export const LOCAL_TOOLS = ['check', 'fmt', 'expand', 'generate', 'highlight', 'share', 'open', 'instruments', 'cheatsheet', 'station'];
export const CAP_TOOLS = ['compose', 'repair', 'store', 'render', 'queue', 'library_list', 'library_save', 'library_get', 'library_delete', 'export', 'jam_send'];

export const PROTOCOL_VERSION = '2025-06-18';
export const SERVER_INFO = Object.freeze({ name: 'beatscript-studio', version: '0.1.0' });

const MAX_TEXT = 65536; // the embed config's source.text maxLength
const ROOM_MAX = 4096; // room.emit carries at most 4 KiB of JSON
const SAMPLE_RESULT_MAX = 32768; // a sample tool's result is at most 32 KB of JSON
const REPAIRS = 2; // compose and repair send check's diagnostics back to Claude at most this often
const CID_PAT = '^z[1-9A-HJ-NP-Za-km-z]{20,100}$';
const DOC_ID = /^(?!\.\.?$)[A-Za-z0-9_.~:@+-]{1,200}$/; // a db path segment

// ---- schemas, in btwserve's style --------------------------------------------------------------------------------

const str = (description, extra = {}) => ({ type: 'string', ...extra, description });
const text = (description = 'The program text.') => str(description, { maxLength: MAX_TEXT });
const schema = (properties = {}, required = []) => ({ type: 'object', properties, required, additionalProperties: false });
const hints = (readOnlyHint, destructiveHint, idempotentHint, openWorldHint) => ({ readOnlyHint, destructiveHint, idempotentHint, openWorldHint });
const READ = hints(true, false, true, false);

// ---- results -----------------------------------------------------------------------------------------------------

const answer = (data) => {
  const obj = data && typeof data === 'object' && !Array.isArray(data) ? data : { result: data ?? null };
  return { content: [{ type: 'text', text: JSON.stringify(obj) }], structuredContent: obj };
};
const failure = (message) => ({ content: [{ type: 'text', text: message }], isError: true });
const pick = (o, keys) => Object.fromEntries(keys.filter((k) => o[k] !== undefined).map((k) => [k, o[k]]));

function strArg(a, key, { required = false, max = MAX_TEXT } = {}) {
  const v = a[key];
  if (v == null || v === '') {
    if (required) throw new Error(`argument "${key}" is required`);
    return null;
  }
  if (typeof v !== 'string') throw new Error(`argument "${key}" is a string`);
  if (v.length > max) throw new Error(`argument "${key}" is longer than ${max} characters`);
  return v;
}
const textArg = (a) => strArg(a, 'text', { required: true });

const errorsOf = (rep) => (rep?.diagnostics || []).filter((d) => d.severity === 'error');
const firstError = (rep) => {
  const d = errorsOf(rep)[0];
  return d ? `the program has errors: ${d.line}:${d.col} ${d.message}` : 'the program has errors';
};
const brief = (d) => ({ severity: d.severity, line: d.line, col: d.col, message: d.message, ...(d.help ? { help: d.help } : {}) });
/** check's answer without the compiled nets: what a model needs, small enough for a sample tool result (32 KB). */
const checkBrief = (rep) => ({ ok: !!rep?.ok, cid: rep?.cid || null, errors: errorsOf(rep).length, diagnostics: (rep?.diagnostics || []).slice(0, 20).map(brief) });
const programName = (t) => /^#!\s*name:\s*(.+?)\s*$/m.exec(t || '')?.[1] || null;

/** A filename without its extension: the one given, else the program's `#! name:`, else "beatscript". */
function baseName(filename, t) {
  const raw = (filename || programName(t) || 'beatscript').replace(/(\.(btw|txt|wav|zip))+$/i, '');
  return raw.replace(/[^\p{L}\p{N}._-]+/gu, '-').replace(/^[-.]+|[-.]+$/g, '').slice(0, 80) || 'beatscript';
}

/**
 * The program in a fenced block of an answer: the last one marked btw or beatscript, else the last of any kind
 * (with tools the answer is every round's text, so the program comes last). null when there is no closed fence.
 */
function fenced(reply) {
  const blocks = [...String(reply).matchAll(/```([^\n`]*)\r?\n([\s\S]*?)```/g)];
  const marked = blocks.filter((b) => /^\s*(btw|beatscript)\s*$/i.test(b[1]));
  return (marked.length ? marked : blocks).at(-1)?.[2] ?? null;
}

/** Only a share link's origin is moved: its path and fragment stay as the module made them. */
const SHARE_PATH = { url: '/#', social_url: '/c/' };
function rehome(link, origin, key) {
  let u;
  try { u = new URL(link); } catch { return link; }
  if (!/^https?:$/.test(u.protocol) || !link.startsWith(u.origin + SHARE_PATH[key])) return link;
  return origin + link.slice(u.origin.length);
}
function originOf(site) {
  try {
    const u = new URL(site);
    return /^https?:$/.test(u.protocol) ? u.origin : null;
  } catch {
    return null;
  }
}

// ---- a WAV in a zip ----------------------------------------------------------------------------------------------
// downloads.save takes only an allowlist of extensions, with neither .btw nor .wav on it: the program is saved as
// .btw.txt, and the WAV inside a zip that stores it uncompressed.

const CRC = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
export function crc32(bytes) {
  let c = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) c = CRC[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** A zip of one file, stored (method 0), dated 1980-01-01 so the same file always gives the same bytes. */
export function zipOne(name, data) {
  const n = new TextEncoder().encode(name);
  const crc = crc32(data);
  const cd = 30 + n.length + data.length;
  const end = cd + 46 + n.length;
  const out = new Uint8Array(end + 22);
  const v = new DataView(out.buffer);
  const u16 = (at, x) => v.setUint16(at, x, true);
  const u32 = (at, x) => v.setUint32(at, x, true);
  // the fields the local header (from 4) and the central entry (from 6, past its "made by") share
  const common = (at) => {
    u16(at, 20); // version needed
    u16(at + 2, 0x0800); // the name is UTF-8
    u16(at + 4, 0); // stored
    u16(at + 6, 0); // time
    u16(at + 8, 0x21); // date
    u32(at + 10, crc);
    u32(at + 14, data.length);
    u32(at + 18, data.length);
    u16(at + 22, n.length);
  };
  u32(0, 0x04034b50);
  common(4);
  out.set(n, 30);
  out.set(data, 30 + n.length);
  u32(cd, 0x02014b50);
  u16(cd + 4, 20); // made by
  common(cd + 6);
  out.set(n, cd + 46); // extra, comment, disk, attributes and the local header's offset stay 0
  u32(end, 0x06054b50);
  u16(end + 8, 1);
  u16(end + 10, 1);
  u32(end + 12, 46 + n.length);
  u32(end + 16, cd);
  return out;
}

// ---- what each capability's failure codes mean to the viewer -------------------------------------------------------

const SAMPLE_WHY = {
  not_granted: 'Claude is not allowed on this page.',
  sampling_disabled: 'Claude is not available for this account.',
  rate_limited: 'Too many requests to Claude just now: try again later.',
  refused: 'Claude declined this request: ask for something else.',
  session_expired: 'Sign in to Claude again.',
  prompt_too_large: 'The program is too long to send to Claude.',
  cancelled: 'Stopped.',
};
const sampleWhy = (e) => SAMPLE_WHY[e?.code] || `Claude did not answer (${e?.code || e?.message || e}).`;

function mcpWhy(e, server, tool) {
  switch (e?.code) {
    case 'needs_reauth': return `Reconnect ${server} in claude.ai Settings, Connectors.`;
    case 'server_not_connected': return `Add the ${server} connector in claude.ai Settings, Connectors.`;
    case 'selection_required': return `Choose which ${server} connector this page uses.`;
    case 'not_in_manifest': case 'blocked_by_policy': return `This page may not call ${server} ${tool}.`;
    case 'approval_required': case 'consent_required': return `${server} ${tool} needs your approval.`;
    case 'server_unavailable': return `${server} did not answer: try again in a moment.`;
    case 'tool_error': return `${server} ${tool}: ${e.message}`;
    default: return `${server} ${tool} failed: ${e?.message || e?.code || e}`;
  }
}

const DB_WHY = {
  quota_exceeded: 'Your library is full: delete a program first.',
  resource_exhausted: 'Too many library requests just now: try again in a moment.',
  invalid_argument: 'This page may not write to your library.',
  revoked: 'This page has lost its store.',
  unavailable: 'Your library could not be reached: try again.',
};

const SAVE_WHY = {
  declined: 'You declined the download.',
  too_large: 'The file is too large to save here.',
  rejected_extension: 'This page may not save that kind of file.',
  extension_not_enabled: 'This page may not save that kind of file.',
  rate_limited: 'A save is already waiting for you: answer it first.',
  unavailable: 'Saving files is not available here.',
};

const needs = (ns) => `needs the ${ns} capability: a Claude artifact that declares it`;

// ---- the surface -------------------------------------------------------------------------------------------------

/**
 * encodeWav(left, right, sampleRate) -> Uint8Array: the WAV encoder export's wav format uses (the player's
 * render-model.js wav). Without it, or without render, export saves only the program text.
 */
export function createToolSurface({ btw, caps = {}, site = 'https://beatscript.app', server = 'Beatscript', render = null, encodeWav = null, onLoad = null, onJam = null } = {}) {
  caps = caps || {};
  const origin = originOf(site);
  const has = (fn) => (typeof btw?.[fn] === 'function' ? null : `this build of the wasm module has no ${fn}`);
  const cap = (ns) => (caps[ns] ? null : needs(ns));
  const noWav = typeof render !== 'function' ? 'A WAV needs the render engine, which this page has not loaded.'
    : typeof encodeWav !== 'function' ? 'A WAV needs the WAV encoder, which this page has not loaded.' : null;
  const formats = noWav ? ['btw'] : ['btw', 'wav'];

  /** Hand a program to the editor; a failing editor does not fail the tool that found the program. */
  const loaded = (t, meta) => {
    if (typeof onLoad !== 'function' || typeof t !== 'string') return;
    try { onLoad(t, meta); } catch (e) { globalThis.reportError?.(e); }
  };

  /**
   * makeFacade's share passes the module no origin, so the module answers links on its own default site. They are
   * moved to the origin of `site`: the CID and the packed text do not depend on it. beats_url stays where it is.
   */
  const onSite = (ans) => {
    if (!ans?.ok || !origin) return ans;
    const out = { ...ans };
    for (const k of Object.keys(SHARE_PATH)) if (typeof out[k] === 'string') out[k] = rehome(out[k], origin, k);
    if (typeof out.url === 'string') out.chars = [...out.url].length; // the module counts characters, not UTF-16 units
    return out;
  };

  // ---- compose and repair: ask Claude, check the answer here, send the problems back ----

  const instruction = (request, current) => {
    const inst = btw.instruments() || {};
    // a name that starts with a digit (808-bass) reads as a sum and cannot be written, so it is not offered
    const names = (k) => (Array.isArray(inst[k]) ? inst[k].filter((x) => /^[A-Za-z]/.test(x)).join(', ') : 'see the instruments tool');
    const parts = [
      'You write BeatScript, a small language for whole music tracks as text. Its grammar, in full:',
      btw.cheatsheet(),
      `Voices you may name: ${names('voices')}.\nKits you may name: ${names('kits')}.`,
      `The request: ${request}`,
    ];
    if (current) parts.push(`The current program:\n\`\`\`btw\n${current.replace(/\n?$/, '\n')}\`\`\``);
    parts.push('Answer with one program in a fenced block (```btw ... ```). If you have the check tool, use it: the program must pass.');
    return parts.join('\n\n');
  };

  const problems = (rep) => {
    const lines = errorsOf(rep).slice(0, 10).map((d) => d.render || `${d.line}:${d.col}: ${d.message}`);
    return `check found these problems:\n\n${lines.join('\n\n')}\n\nFix them and answer with the whole program in one fenced block.`;
  };
  const NO_FENCE = 'Your answer had no fenced program. Answer with the whole program in one fenced block.';

  /**
   * Ask Claude: the first call is the instruction itself, a repair round the whole exchange so far. The page's tools
   * go along only where limits() says this view runs them (elsewhere the call rejects tools_unavailable). cache is
   * false so that asking again really asks again, rather than replaying the same failed answers for five minutes.
   */
  const ask = async (turns) => {
    const sample = caps.sample;
    const lim = typeof sample.limits === 'function' ? await sample.limits().catch(() => null) : null;
    const tools = lim?.tools ? surface.asSampleTools(['check', 'fmt', 'instruments']).slice(0, lim.tools.maxCount ?? 3) : [];
    try {
      return await sample(turns.length === 1 ? turns[0].content : turns, tools.length ? { tools, cache: false } : { cache: false });
    } catch (e) {
      throw new Error(sampleWhy(e));
    }
  };

  /** Up to `calls` answers from Claude; the first that checks wins. rounds: the calls it took. */
  const converse = async (turns, calls, via) => {
    let last = null;
    let rep = null;
    for (let round = 1; round <= calls; round++) {
      const { text: reply } = await ask(turns);
      let prog = fenced(reply);
      if (prog == null && typeof reply === 'string' && btw.check(reply).ok) prog = reply; // a bare program, no fence
      rep = prog == null ? null : btw.check(prog);
      if (rep?.ok) {
        const f = btw.fmt(prog);
        const out = f?.ok && typeof f.text === 'string' ? f.text : prog;
        loaded(out, { cid: rep.cid, via });
        return { ok: true, text: out, cid: rep.cid, diagnostics: checkBrief(rep).diagnostics, rounds: round };
      }
      last = prog ?? last;
      turns = [...turns, { role: 'assistant', content: reply }, { role: 'user', content: prog == null ? NO_FENCE : problems(rep) }];
    }
    return { ok: false, text: last, cid: null, diagnostics: rep ? checkBrief(rep).diagnostics : [], rounds: calls };
  };

  // ---- the BeatScript connector, the library, downloads ----

  const connector = async (tool, input) => {
    try {
      const r = await caps.mcp.callTool(server, tool, input);
      return r?.payload ?? r?.structuredContent ?? null;
    } catch (e) {
      throw new Error(mcpWhy(e, server, tool));
    }
  };
  const checked = (t) => {
    const rep = btw.check(t);
    if (!rep.ok) throw new Error(firstError(rep));
    return rep;
  };

  /** The viewer's library: data/users/<id>/library is their document, its programs a collection below it. */
  const library = async () => {
    const uid = await caps.user.id();
    if (!uid) throw new Error('Your library needs you signed in to Claude.');
    return caps.db.doc(`data/users/${uid}/library`).collection('programs');
  };
  const inLibrary = async (fn) => {
    try { return await fn(await library()); } catch (e) { throw new Error(DB_WHY[e?.code] || e?.message || String(e)); }
  };
  const docId = (a) => {
    const id = strArg(a, 'id', { required: true, max: 200 });
    if (!DOC_ID.test(id)) throw new Error('no program with that id is in your library');
    return id;
  };

  // ---- the tools ----

  const T = {
    check: {
      title: 'Check a BeatScript program', via: 'wasm', annotations: READ, needs: () => has('check'),
      description: 'Check a program in the page: ok, the errors and warnings at line:col with help, the CID of its canonical text, tempo, key, sections and seconds.',
      inputSchema: schema({ text: text('The program text. Comments and blank lines are fine.') }, ['text']),
      run: (a) => btw.check(textArg(a)),
    },
    fmt: {
      title: 'Format a BeatScript program', via: 'wasm', annotations: READ, needs: () => has('fmt'),
      description: 'The program formatted as the site stores it, with its #! header lines and comments kept, and its CID. A program with errors gives the diagnostics.',
      inputSchema: schema({ text: text() }, ['text']),
      run: (a) => btw.fmt(textArg(a)),
    },
    expand: {
      title: "Write out a program's seeds and templates", via: 'wasm', annotations: READ, needs: () => has('expand'),
      description: 'The program with its seed(...), template(...) and gen@N references written out as plain BeatScript: text, its cid, from_cid (the program as given) and refs.',
      inputSchema: schema({
        text: text('The program text, with gen@N calls, seed(...) or template(...) references in it.'),
        lineage: { type: 'boolean', description: 'Open the written-out text with `#! forked-from: <CID of the program as given>` (default false).' },
      }, ['text']),
      run: (a) => btw.expand(textArg(a), a.lineage === true),
    },
    generate: {
      title: 'Generate a track from a seed (no model)', via: 'wasm', annotations: READ, needs: () => has('generate'),
      description: 'The seeded generator: a whole program for a genre, the same inputs always giving the same text and CID. Nothing is stored. The program goes into the editor.',
      inputSchema: schema({
        genre: str('The genre whose rhythms, chords, instruments and song structure it uses: techno, house, jazz, dnb, ... (default techno).', { maxLength: 32 }),
        bpm: { type: 'number', minimum: 40, maximum: 300, description: "Tempo; default the genre's." },
        key: str("Key: am, c, f#m, d:dorian ...; default the genre's.", { maxLength: 16 }),
        length: { type: 'string', enum: ['short', 'standard', 'long'], description: 'Song length (default standard).' },
        seed: { type: ['string', 'integer'], description: 'A number or a word (default 1). Same inputs, same program text and CID.' },
        generator: { type: 'integer', enum: [1, 2], description: 'The version of the seeded generator: 1 (seeded@1, frozen) or 2 (seeded@2).' },
      }),
      run: (a) => {
        const g = btw.generate(pick(a, ['genre', 'bpm', 'key', 'length', 'seed', 'generator']));
        if (g?.ok) loaded(g.text, { cid: g.cid, name: g.name, via: 'generate' });
        return g;
      },
    },
    highlight: {
      title: 'Highlight a program', via: 'wasm', annotations: READ, needs: () => has('highlight'),
      description: 'The highlighter spans of a program, as [start, end, class] with UTF-16 offsets: keyword, deck, role, step, rest, number, comment and so on.',
      inputSchema: schema({ text: text() }, ['text']),
      run: (a) => ({ spans: btw.highlight(textArg(a)) }),
    },
    share: {
      title: 'Make share links', via: 'wasm', annotations: READ, needs: () => has('share'),
      description: `Self-sealing share links for a program that checks (the program travels in the link, on ${origin || 'the site'}) and its CID. Nothing is stored: store does that.`,
      inputSchema: schema({ text: text('The program text. It must pass check.') }, ['text']),
      run: (a) => onSite(btw.share(textArg(a))),
    },
    open: {
      title: 'Open a share link', via: 'wasm', annotations: READ, needs: () => has('open'),
      description: 'Open a self-sealing share link in the page and verify it: the CID, the canonical text and whether the text is that CID. A bare CID has no text. The program goes into the editor.',
      inputSchema: schema({ url: str('A share link (self-sealing #<CID>.<packed>), a #fragment, <CID>.<packed>, or a bare CID.', { maxLength: 32768 }) }, ['url']),
      run: (a) => {
        const o = btw.open(strArg(a, 'url', { required: true, max: 32768 }));
        if (o?.ok && o.verified) loaded(o.canonical, { cid: o.cid, via: 'open' });
        return o;
      },
    },
    instruments: {
      title: 'Voices, kits and palettes', via: 'wasm', annotations: READ, needs: () => has('instruments'),
      description: 'The voices, families, kits and palettes a program may name.',
      inputSchema: schema(),
      run: () => btw.instruments(),
    },
    cheatsheet: {
      title: 'The language cheat sheet', via: 'wasm', annotations: READ, needs: () => has('cheatsheet'),
      description: "BeatScript's compact grammar, as Markdown: read it before writing a program.",
      inputSchema: schema(),
      // Markdown, so the text block is the sheet itself rather than its JSON string
      run: () => {
        const md = btw.cheatsheet();
        return { content: [{ type: 'text', text: md }], structuredContent: { cheatsheet: md } };
      },
    },
    station: {
      title: "A radio station's tracks", via: 'wasm', annotations: READ, needs: () => has('station'),
      description: "A radio station's tracks: position n always gives the same track. The answer is the station, its tracks from n and their CIDs.",
      inputSchema: schema({
        station: str('The station: its text (`station { from ... next ... }`).', { maxLength: MAX_TEXT }),
        n: { type: 'integer', minimum: 0, description: 'The position to start at (default 0).' },
        count: { type: 'integer', minimum: 1, maximum: 16, description: 'How many tracks (default 1).' },
      }, ['station']),
      run: (a) => btw.station({ station: strArg(a, 'station', { required: true }), ...pick(a, ['n', 'count']) }),
    },

    compose: {
      title: 'Compose a program with Claude', via: 'sample', annotations: hints(false, false, false, true), needs: () => cap('sample'),
      description: 'Ask Claude for a program, or for a change to the current one; the answer is checked here and its problems sent back at most twice. Answers {ok, text, cid, diagnostics, rounds}; a program that checks goes into the editor.',
      inputSchema: schema({ prompt: str('What to write, or what to change.', { maxLength: 4000 }), text: text('Optional: the current program, to change.') }, ['prompt']),
      run: (a) => {
        const prompt = strArg(a, 'prompt', { required: true, max: 4000 });
        return converse([{ role: 'user', content: instruction(prompt, strArg(a, 'text')) }], 1 + REPAIRS, 'compose');
      },
    },
    repair: {
      title: 'Repair a program with Claude', via: 'sample', annotations: hints(false, false, false, true), needs: () => cap('sample'),
      description: "Send a program and check's problems to Claude until it checks (at most twice). Answers {ok, text, cid, diagnostics, rounds}; a program that already checks takes no rounds.",
      inputSchema: schema({ text: text() }, ['text']),
      run: async (a) => {
        const t = textArg(a);
        const rep = btw.check(t);
        if (rep.ok) return { ok: true, text: t, cid: rep.cid, diagnostics: checkBrief(rep).diagnostics, rounds: 0 };
        const turns = [{ role: 'user', content: instruction('Fix this program so that it passes check, and keep its music.', t) }, { role: 'user', content: problems(rep) }];
        return converse(turns, REPAIRS, 'repair');
      },
    },
    store: {
      title: 'Store a program on BeatScript', via: 'mcp', annotations: hints(false, false, true, true), needs: () => cap('mcp'),
      description: `Store a program that checks through your ${server} connector (its share tool): signed in, it is kept on the site with its rights and a short link.`,
      inputSchema: schema({
        text: text('The program text. It must pass check; it is stored publicly.'),
        name: str("A name for the program (else its `#! name:` line).", { maxLength: 80 }),
        license: str('A Creative Commons SPDX id (default CC0-1.0).', { maxLength: 40 }),
      }, ['text']),
      run: (a) => {
        const t = textArg(a);
        checked(t);
        return connector('share', { text: t, ...pick(a, ['name', 'license']) });
      },
    },
    render: {
      title: 'Render audio on BeatScript', via: 'mcp', annotations: hints(false, false, true, true), needs: () => cap('mcp'),
      description: `Render a program to audio on the site through your ${server} connector: give text or cid. Answers {cid, stream_url, status_url, seconds, format}.`,
      inputSchema: schema({
        text: text('The program text. It must pass check; it is stored publicly (CC0-1.0).'),
        cid: str('CID of a program already stored.', { maxLength: 100, pattern: CID_PAT }),
        format: { type: 'string', enum: ['opus', 'wav', 'midi', 'stems', 'stems-opus'], default: 'opus', description: 'opus or wav to listen; midi, stems or stems-opus for a DAW.' },
      }),
      run: (a) => {
        const t = strArg(a, 'text');
        if (t) checked(t);
        else if (!strArg(a, 'cid', { max: 100 })) throw new Error('give text or cid');
        return connector('render', pick(a, ['text', 'cid', 'format']));
      },
    },
    queue: {
      title: 'Up next on a BeatScript tab', via: 'mcp', annotations: hints(false, true, false, true), needs: () => cap('mcp'),
      description: `The "up next" list of stored programs on one of your BeatScript tabs, through your ${server} connector: list, add, remove, clear or skip.`,
      inputSchema: {
        type: 'object',
        properties: {
          action: { type: 'string', enum: ['list', 'add', 'remove', 'clear', 'skip'], description: 'list, add, remove, clear or skip.' },
          items: { type: 'array', maxItems: 20, items: { type: 'object' }, description: 'add: the programs to append, each with exactly one of cid, example, url, and optionally bars, minutes, title.' },
          item: str('remove: an item id from list (q1, q2, ...) or a CID.', { maxLength: 100 }),
          session: str('Session id of one browser tab.', { maxLength: 64 }),
          device: str('Id of one of your paired phones.', { maxLength: 64 }),
        },
        required: ['action'],
      },
      run: (a) => connector('queue', a), // the connector checks its own arguments
    },
    library_list: {
      title: 'Your library', via: 'db', annotations: READ, needs: () => cap('db') || cap('user'),
      description: 'The programs in your private library, newest first: {programs: [{id, name, cid, savedAt}]}.',
      inputSchema: schema(),
      run: () => inLibrary(async (col) => {
        const snap = await col.orderBy('savedAt', 'desc').limit(200).get();
        return { programs: snap.docs.map((d) => ({ id: d.id, ...pick(d.data() || {}, ['name', 'cid', 'savedAt']) })) };
      }),
    },
    library_save: {
      title: 'Save to your library', via: 'db', annotations: hints(false, false, true, false), needs: () => cap('db') || cap('user'),
      description: 'Save a program to your private library. A program that checks is kept under its CID, so saving it again replaces it; a draft with errors gets an id of its own.',
      inputSchema: schema({ text: text(), name: str("A name (else its `#! name:` line).", { maxLength: 80 }) }, ['text']),
      run: (a) => {
        const t = textArg(a);
        const rep = btw.check(t);
        const doc = { name: strArg(a, 'name', { max: 80 }) || programName(t) || 'untitled', text: t, cid: rep.ok ? rep.cid : null, savedAt: new Date().toISOString() };
        return inLibrary(async (col) => {
          const ref = doc.cid ? col.doc(doc.cid) : col.doc();
          await ref.set(doc);
          return { id: ref.id, name: doc.name, cid: doc.cid, savedAt: doc.savedAt };
        });
      },
    },
    library_get: {
      title: 'Open from your library', via: 'db', annotations: READ, needs: () => cap('db') || cap('user'),
      description: 'One program from your library: {id, name, text, cid, savedAt}. It goes into the editor.',
      inputSchema: schema({ id: str('The id, from library_list.', { maxLength: 200 }) }, ['id']),
      run: (a) => {
        const id = docId(a);
        return inLibrary(async (col) => {
          const d = await col.doc(id).get();
          if (!d.exists) throw new Error('no program with that id is in your library');
          const out = { id, ...pick(d.data() || {}, ['name', 'text', 'cid', 'savedAt']) };
          loaded(out.text, { cid: out.cid ?? null, name: out.name, via: 'library' });
          return out;
        });
      },
    },
    library_delete: {
      title: 'Delete from your library', via: 'db', annotations: hints(false, true, true, false), needs: () => cap('db') || cap('user'),
      description: 'Delete one program from your library, by its id.',
      inputSchema: schema({ id: str('The id, from library_list.', { maxLength: 200 }) }, ['id']),
      run: (a) => {
        const id = docId(a);
        return inLibrary(async (col) => {
          await col.doc(id).delete();
          return { deleted: id };
        });
      },
    },
    export: {
      title: 'Save a file', via: 'downloads', annotations: hints(false, false, false, false), needs: () => cap('downloads'),
      description: `Offer the program as a file: btw saves the text (name.btw.txt)${noWav ? '' : ', wav renders it here and saves the WAV in a zip (name.wav.zip)'}. You confirm the save.`,
      inputSchema: schema({
        format: { type: 'string', enum: formats, description: noWav ? 'btw: the program text.' : 'btw: the program text; wav: the audio, rendered in the page.' },
        text: text(),
        filename: str("The file's name without extension (else the program's `#! name:`).", { maxLength: 200 }),
      }, ['format', 'text']),
      run: async (a) => {
        if (a.format === 'wav' && noWav) throw new Error(noWav);
        if (a.format !== 'btw' && a.format !== 'wav') throw new Error(`argument "format" is ${formats.join(' or ')}`);
        const t = textArg(a);
        const base = baseName(strArg(a, 'filename', { max: 200 }), t);
        let file;
        if (a.format === 'btw') file = { filename: `${base}.btw.txt`, data: t };
        else {
          checked(t);
          const r = await render(t);
          file = { filename: `${base}.wav.zip`, data: zipOne(`${base}.wav`, encodeWav(r.left, r.right, r.sampleRate)) };
        }
        const bytes = typeof file.data === 'string' ? new TextEncoder().encode(file.data).length : file.data.length;
        try {
          await caps.downloads.save(file);
        } catch (e) {
          throw new Error(SAVE_WHY[e?.code] || e?.message || 'Downloads are not available here.');
        }
        return { saved: file.filename, bytes };
      },
    },
    jam_send: {
      title: 'Send a program to the room', via: 'room', annotations: hints(false, false, false, true), needs: () => cap('room'),
      description: 'Send a program that checks to everyone who has this page open (the room): their studios check it again before they take it.',
      inputSchema: schema({ text: text('The program text (at most 4 KiB with its CID).') }, ['text']),
      run: async (a) => {
        const t = textArg(a);
        const rep = checked(t);
        const data = { text: t, cid: rep.cid };
        if (new TextEncoder().encode(JSON.stringify(data)).length > ROOM_MAX) throw new Error('The program is too long for the room: 4 KiB at most.');
        try {
          await caps.room.emit('program', data);
        } catch (e) {
          throw new Error(e?.code === 'not_permitted' ? "Only this page's editors may send programs to the room." : e?.message || String(e));
        }
        return { sent: true, cid: rep.cid };
      },
    },
  };

  // A received program is untrusted: it is checked here, and only one that checks reaches onJam, with the CID this
  // page computed (not the one it claims). Our own sends come back too (sameTab); the editor already has those.
  const unhear = caps.room && typeof onJam === 'function'
    ? caps.room.on('program', (msg) => {
      const t = msg?.data?.text;
      if (msg?.sameTab || typeof t !== 'string' || !t || t.length > MAX_TEXT) return;
      let rep;
      try { rep = btw.check(t); } catch { return; }
      if (!rep?.ok) return;
      // a failing handler is the page's to hear about, not the room's
      try { onJam({ topic: 'program', text: t, cid: rep.cid, peer: msg.peer, by: msg.by ?? null, isMe: !!msg.isMe, guest: !!msg.guest }); } catch (e) { globalThis.reportError?.(e); }
    })
    : null;

  const surface = {
    list() {
      return Object.entries(T).map(([name, t]) => {
        const why = t.needs();
        return { name, title: t.title, description: t.description, inputSchema: t.inputSchema, annotations: { title: t.title, ...t.annotations }, via: t.via, available: !why, why };
      });
    },

    async call(name, args) {
      try {
        const t = typeof name === 'string' && Object.hasOwn(T, name) ? T[name] : null;
        if (!t) return failure(`no tool named ${typeof name === 'string' ? JSON.stringify(name) : typeof name}`);
        const why = t.needs();
        if (why) return failure(why);
        const a = args ?? {};
        if (typeof a !== 'object' || Array.isArray(a)) return failure('the arguments are a JSON object');
        if (t.inputSchema.additionalProperties === false) {
          const extra = Object.keys(a).find((k) => !Object.hasOwn(t.inputSchema.properties, k));
          if (extra) return failure(`${name} has no argument "${extra}"`);
        }
        const out = await t.run(a);
        return Array.isArray(out?.content) ? out : answer(out);
      } catch (e) {
        return failure(e?.message || String(e));
      }
    },

    asSampleTools(names = LOCAL_TOOLS) {
      return surface.list().filter((t) => names.includes(t.name) && t.available).map((t) => ({
        name: t.name,
        description: t.description,
        inputSchema: t.inputSchema,
        // a problem is thrown, so Claude reads "Error: <why>" and carries on
        execute: async (input) => {
          const r = await surface.call(t.name, input);
          if (r.isError) throw new Error(r.content[0].text);
          const sc = r.structuredContent;
          // check's nets and fmt's second copy of the text (canonical) are not what Claude needs, and the result is capped
          const out = t.name === 'check' ? checkBrief(sc) : t.name === 'fmt' ? pick(sc, ['ok', 'cid', 'text', 'diagnostics']) : sc;
          if (JSON.stringify(out).length > SAMPLE_RESULT_MAX) throw new Error(`the answer is over ${SAMPLE_RESULT_MAX >> 10} KB, more than a tool result carries`);
          return out;
        },
      }));
    },

    async handle(msg) {
      const raw = msg && typeof msg === 'object' ? msg.id : null;
      const id = typeof raw === 'string' || typeof raw === 'number' ? raw : null; // an id of another type is not echoed
      const fail = (code, message) => ({ jsonrpc: '2.0', id, error: { code, message } });
      if (!msg || typeof msg !== 'object' || msg.jsonrpc !== '2.0' || typeof msg.method !== 'string') return fail(-32600, 'not a JSON-RPC 2.0 request');
      if (!Object.hasOwn(msg, 'id')) return null; // a notification (notifications/initialized ...) has no answer
      if (raw !== id) return fail(-32600, 'the id is a string or a number');
      const { method, params } = msg;
      const ok = (result) => ({ jsonrpc: '2.0', id, result });
      try {
        switch (method) {
          case 'initialize':
            return ok({ protocolVersion: PROTOCOL_VERSION, capabilities: { tools: {} }, serverInfo: { ...SERVER_INFO } });
          case 'ping':
            return ok({});
          case 'tools/list':
            // an MCP client is shown only what it can call; list() also says what is missing and why
            return ok({ tools: surface.list().filter((t) => t.available).map(({ name, title, description, inputSchema, annotations }) => ({ name, title, description, inputSchema, annotations })) });
          case 'tools/call':
            if (typeof params?.name !== 'string' || !Object.hasOwn(T, params.name)) return fail(-32602, `unknown tool: ${String(params?.name)}`);
            return ok(await surface.call(params.name, params.arguments ?? {}));
          default:
            return fail(-32601, `method not found: ${method}`);
        }
      } catch (e) {
        return fail(-32603, e?.message || String(e));
      }
    },

    serve(win, { origins = ['*'] } = {}) {
      // The default, ['*'], answers ANY window that can post a message to this one: the page that embeds the studio,
      // a sibling frame on that page, a page that opened it. Each of them can list and call every tool, the capability
      // tools included (compose spends the viewer's Claude usage, library_delete deletes from their library), as far
      // as this viewer allowed this page those capabilities. Pass the parent's origin to answer that page alone.
      const allowed = [].concat(origins).filter((o) => typeof o === 'string');
      const any = allowed.includes('*');
      const onMessage = (event) => {
        const msg = event.data;
        if (!msg || typeof msg !== 'object' || msg.jsonrpc !== '2.0' || typeof msg.method !== 'string' || !event.source) return;
        // the opaque origin "null" (a sandboxed frame) could be any page, so a list of origins never admits it
        if (!any && (event.origin === 'null' || !allowed.includes(event.origin))) return;
        surface.handle(msg).then((res) => {
          // with '*', a sandboxed sender, which postMessage cannot name, is answered with '*'; it is still only event.source
          if (res) event.source.postMessage(res, event.origin && event.origin !== 'null' ? event.origin : '*');
        }).catch((e) => globalThis.reportError?.(e));
      };
      win.addEventListener('message', onMessage);
      return () => win.removeEventListener('message', onMessage);
    },

    /** Stop hearing the room. */
    close() {
      unhear?.();
    },
  };
  return surface;
}
