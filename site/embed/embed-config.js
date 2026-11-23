// The embed config: one description of an embedded BeatScript program that every surface reads (the <beatscript-track>
// player, the <beatscript-studio> editor, a card, plain audio, the btwserve `embed` tool). The contract is
// site/web/embed-config.schema.json; SCHEMA below is the same object (a test checks it), validated by hand here so the
// page needs no library. btwserve mirrors normalize, toAttributes and artifactCapabilities in Rust, held to
// site/testdata/embed-config/cases.json, so their output is exact: keys in a fixed order, arrays in schema order.
//
//   const raw = fromElement(el);                 // the config JSON, then single attributes over it
//   const { ok, config, errors } = normalize(raw);
//   const host = await detectHost(window);       // what this page can do
//   const { config: effective, notes } = resolve(config, host);

export const SCHEMA = deepFreeze({
  $schema: 'https://json-schema.org/draft/2020-12/schema',
  $id: 'https://beatscript.app/embed-config.schema.json',
  title: 'BeatScript embed config',
  description: 'One config for every way BeatScript is embedded: the <beatscript-track> player, the <beatscript-studio> editor, a static card and plain audio, on a Claude artifact, any web page, an iframe, oEmbed, a README, a notebook, an editor\'s preview or the Android app. Every surface reads it: as JSON (the `config` attribute or a child <script type="application/json" class="beatscript-config">), as single attributes (which override it), or as the btwserve `embed` MCP tool\'s `config` argument. A surface that cannot do something the config asks for says so and does what it can (docs/btw/embed.md).',
  type: 'object',
  additionalProperties: false,
  properties: {
    $schema: { type: 'string', description: 'This schema\'s URL, if the author wants an editor to check the config.' },
    v: { const: 1, default: 1, description: 'The config\'s own version.' },
    kind: {
      enum: ['player', 'studio', 'card', 'audio'],
      default: 'player',
      description: 'player: a card with a Play button (<beatscript-track>). studio: the editor, its checker and the player in one element (<beatscript-studio>); the deterministic tools run in the page. card: a static card (name, CID, rights, links), no script and no sound. audio: a plain <audio> of the server\'s render.',
    },
    source: {
      description: 'What plays. Exactly one of text, cid, set or station; cid may go with text, as the CID the text must have (a text that is not that program is refused).',
      type: 'object',
      additionalProperties: false,
      properties: {
        text: { type: 'string', maxLength: 65536, description: 'The program text.' },
        cid: { type: 'string', pattern: '^z[1-9A-HJ-NP-Za-km-z]{20,120}$', description: 'A program\'s CID: with text, the CID it must have; alone, a program stored on the site (a page that may fetch nothing from the site needs the text too).' },
        set: {
          type: 'array',
          minItems: 1,
          maxItems: 64,
          description: 'A set or playlist, played in order.',
          items: {
            type: 'object',
            additionalProperties: false,
            properties: {
              text: { type: 'string', maxLength: 65536 },
              cid: { type: 'string', pattern: '^z[1-9A-HJ-NP-Za-km-z]{20,120}$' },
              title: { type: 'string', maxLength: 80 },
            },
            anyOf: [{ required: ['text'] }, { required: ['cid'] }],
          },
        },
        station: { type: 'string', minLength: 1, maxLength: 64, description: 'A radio station\'s name: position n always gives the same track (/radio).' },
        n: { type: 'integer', minimum: 0, default: 0, description: 'With station: the position to start at.' },
        count: { type: 'integer', minimum: 1, maximum: 64, default: 8, description: 'With station: how many tracks to queue.' },
      },
      oneOf: [
        { required: ['text'], not: { anyOf: [{ required: ['set'] }, { required: ['station'] }] } },
        { required: ['cid'], not: { anyOf: [{ required: ['text'] }, { required: ['set'] }, { required: ['station'] }] } },
        { required: ['set'], not: { anyOf: [{ required: ['text'] }, { required: ['cid'] }, { required: ['station'] }] } },
        { required: ['station'], not: { anyOf: [{ required: ['text'] }, { required: ['cid'] }, { required: ['set'] }] } },
      ],
    },
    audio: {
      enum: ['live', 'render', 'inline', 'url'],
      description: 'How it sounds. live: the wasm engine plays as it goes (the default for player and studio). render: the engine renders one whole pass in the page first, then plays it with a seek bar. inline: the site\'s Ogg Opus render carried in the page as a data: URI (no script, no engine; a stored program only; the default for audio). url: the site\'s render by URL (<audio src>, an RSS enclosure; the host fetches it from the site).',
    },
    features: {
      type: 'array',
      uniqueItems: true,
      items: { enum: ['highlight', 'editor', 'mixer', 'knobs', 'download', 'share', 'compose'] },
      description: 'What the surface shows. highlight: the program, highlighted, with Copy. editor: the text is editable and re-checked as it is typed (studio). mixer: deck levels and mutes. knobs: the program\'s control-deck knobs as sliders. download: save the .btw text and a WAV. share: the self-sealing link and the CID. compose: ask a model for a program or a change (needs the sample capability). Defaults by kind: player [share], studio [highlight, editor, share, download, compose], card [share], audio [download].',
    },
    capabilities: {
      type: 'array',
      uniqueItems: true,
      items: { enum: ['sample', 'mcp', 'db', 'downloads', 'room', 'dictation', 'tts'] },
      description: 'Host capabilities to use when the host allows them (a Claude artifact\'s claude.use()); anything else works without them. sample: compose and repair ask Claude. mcp: store, share, render and queue through the viewer\'s BeatScript connector. db: a library of programs and playlists. downloads: save WAV and .btw. room: a live jam with everyone who has the page open. dictation: speak a compose prompt. tts: read the problems aloud. Defaults by kind: player [downloads], studio [sample, mcp, db, downloads], card and audio [].',
    },
    theme: { enum: ['dark', 'light', 'auto'], default: 'dark', description: 'auto follows prefers-color-scheme.' },
    compact: { type: 'boolean', default: false, description: 'The small form: an 80-pixel player, a studio without the side panel.' },
    width: { type: 'integer', minimum: 80, maximum: 4096, description: 'Width in CSS pixels, where the host needs one (an iframe, oEmbed).' },
    height: { type: 'integer', minimum: 40, maximum: 4096, description: 'Height in CSS pixels, where the host needs one.' },
    title: { type: 'string', maxLength: 80, description: 'The name shown and the page\'s title (default the program\'s `#! name:`).' },
    site: { type: 'string', format: 'uri', pattern: '^https://', default: 'https://beatscript.app', description: 'Where plain links point and what the server-backed forms (iframe, card, audio) are served from.' },
    bridge: { type: 'boolean', default: false, description: 'studio: answer MCP-shaped JSON-RPC (tools/list, tools/call) from the parent frame over postMessage.' },
    pin: {
      type: 'object',
      additionalProperties: false,
      description: 'For generators that emit script tags: the exact package version to load and whether to give each tag its Subresource Integrity hash (from the lock file the generator keeps).',
      properties: {
        version: { type: 'string', pattern: '^[0-9]+\\.[0-9]+\\.[0-9]+(-[0-9A-Za-z.-]+)?$' },
        sri: { type: 'boolean', default: true },
      },
    },
  },
});

const P = SCHEMA.properties;
export const KINDS = P.kind.enum;
export const AUDIO = P.audio.enum;
export const FEATURES = P.features.items.enum;
export const CAPABILITIES = P.capabilities.items.enum;
export const THEMES = P.theme.enum;
export const DEFAULT_SITE = P.site.default;

const N_DEFAULT = P.source.properties.n.default;
const COUNT_DEFAULT = P.source.properties.count.default;

// In FEATURES and CAPABILITIES order, as normalize gives them.
const DEFAULTS = {
  player: { audio: 'live', features: ['share'], capabilities: ['downloads'] },
  studio: { audio: 'live', features: ['highlight', 'editor', 'download', 'share', 'compose'], capabilities: ['sample', 'mcp', 'db', 'downloads'] },
  card: { audio: 'url', features: ['share'], capabilities: [] },
  audio: { audio: 'inline', features: ['download'], capabilities: [] },
};

/** The audio, features and capabilities a kind has when the config does not say (an unknown kind gives the player's). */
export function defaultsFor(kind) {
  // own keys only: a kind such as "constructor" or "__proto__" is unknown, not Object's
  const d = Object.hasOwn(DEFAULTS, kind) ? DEFAULTS[kind] : DEFAULTS.player;
  return { audio: d.audio, features: [...d.features], capabilities: [...d.capabilities] };
}

// ---- the validator: exactly the keywords SCHEMA uses ----

const PATTERN_SAYS = new Map([
  [P.source.properties.cid.pattern, 'is not a program CID'],
  [P.site.pattern, 'must start with https://'],
  [P.pin.properties.version.pattern, 'is not a version such as 1.2.3'],
]);

const TYPE_SAYS = { object: 'an object', array: 'an array', string: 'a string', integer: 'an integer', boolean: 'true or false' };

function isObject(v) {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

function hasType(type, v) {
  if (type === 'object') return isObject(v);
  if (type === 'array') return Array.isArray(v);
  if (type === 'integer') return Number.isInteger(v);
  return typeof v === type;
}

/** "text, cid, set or station" from the branches' required names. */
function alternatives(branches, last) {
  const names = branches.map((b) => (b.required || []).join(' and '));
  return names.length < 2 ? names.join('') : `${names.slice(0, -1).join(', ')} ${last} ${names[names.length - 1]}`;
}

/**
 * The string's length in characters (code points, as JSON Schema counts them, not UTF-16 units), counted only up to
 * cap + 1: that is enough to compare with the limits, and a huge string costs no more than a short one.
 */
function codePoints(v, cap) {
  let n = 0;
  for (const _ of v) if (++n > cap) break;
  return n;
}

// Rust orders strings by code point (their UTF-8 bytes); JS's < compares UTF-16 units, which puts U+10000 and above
// before U+E000..U+FFFF. Lifting the surrogates above that range gives code point order, so the error paths sort alike.
const unit = (c) => (c >= 0xe000 ? c - 0x800 : c >= 0xd800 ? c + 0x2000 : c);
function byCodePoint(a, b) {
  for (let i = 0; i < a.length && i < b.length; i++) {
    const d = unit(a.charCodeAt(i)) - unit(b.charCodeAt(i));
    if (d) return d;
  }
  return a.length - b.length;
}

function matches(s, v) {
  const errs = [];
  check(s, v, '', errs);
  return errs.length === 0;
}

/**
 * Appends {path, message} for each way `v` breaks `s`. At one value, the first failing scalar keyword is its only error
 * (a value of the wrong type is not also too long); an object's or array's members are checked one by one.
 */
function check(s, v, path, errs) {
  const fail = (message) => { errs.push({ path, message }); };
  if (s.type && !hasType(s.type, v)) return fail(`must be ${TYPE_SAYS[s.type]}`);
  if ('const' in s && v !== s.const) return fail(`must be ${JSON.stringify(s.const)}`);
  if (s.enum && !s.enum.includes(v)) return fail(`must be one of ${s.enum.join(', ')}`);
  if (typeof v === 'string') {
    const len = codePoints(v, s.maxLength ?? s.minLength ?? 0);
    if (s.minLength !== undefined && len < s.minLength) return fail(s.minLength === 1 ? 'must not be empty' : `must be at least ${s.minLength} characters`);
    if (s.maxLength !== undefined && len > s.maxLength) return fail(`must be at most ${s.maxLength} characters`);
    if (s.pattern !== undefined && !new RegExp(s.pattern, 'u').test(v)) return fail(PATTERN_SAYS.get(s.pattern) || `must match ${s.pattern}`);
  }
  if (typeof v === 'number') {
    if (s.minimum !== undefined && v < s.minimum) return fail(`must be at least ${s.minimum}`);
    if (s.maximum !== undefined && v > s.maximum) return fail(`must be at most ${s.maximum}`);
  }
  if (Array.isArray(v)) {
    if (s.minItems !== undefined && v.length < s.minItems) return fail(`must have at least ${s.minItems} item${s.minItems === 1 ? '' : 's'}`);
    if (s.maxItems !== undefined && v.length > s.maxItems) return fail(`must have at most ${s.maxItems} items`);
    if (s.uniqueItems) {
      const seen = new Set();
      for (const item of v) {
        const key = JSON.stringify(item);
        if (seen.has(key)) return fail(`has ${key} twice`);
        seen.add(key);
      }
    }
    if (s.items) v.forEach((item, i) => check(s.items, item, `${path}/${i}`, errs));
  }
  if (isObject(v)) {
    for (const [k, val] of Object.entries(v)) {
      const sub = s.properties && Object.hasOwn(s.properties, k) ? s.properties[k] : null;
      const at = `${path}/${k.replace(/~/g, '~0').replace(/\//g, '~1')}`;
      if (sub) check(sub, val, at, errs);
      else if (s.additionalProperties === false) errs.push({ path: at, message: 'is not a known key' });
    }
    for (const k of s.required || []) if (!Object.hasOwn(v, k)) fail(`needs ${k}`);
  }
  if (s.anyOf && !s.anyOf.some((b) => matches(b, v))) fail(`needs ${alternatives(s.anyOf, 'or')}`);
  if (s.oneOf && s.oneOf.filter((b) => matches(b, v)).length !== 1) fail(`needs exactly one of ${alternatives(s.oneOf, 'or')}`);
  if (s.not && matches(s.not, v)) fail('is not allowed here');
}

// ---- normalize ----

const inOrder = (list, all) => all.filter((x) => list.includes(x));

function normSource(s) {
  if (s.set) {
    return { set: s.set.map((it) => {
      const o = {};
      for (const k of ['text', 'cid', 'title']) if (it[k] !== undefined) o[k] = it[k];
      return o;
    }) };
  }
  if (s.station !== undefined) return { station: s.station, n: s.n ?? N_DEFAULT, count: s.count ?? COUNT_DEFAULT };
  const o = {};
  if (s.text !== undefined) o.text = s.text;
  if (s.cid !== undefined) o.cid = s.cid;
  return o;
}

function loneCid(source) {
  return !!source && source.cid !== undefined && source.text === undefined;
}

/**
 * The config, checked against SCHEMA and completed with the kind's defaults: {ok, config, errors, warnings}. input is
 * an object, a JSON string or null (all defaults). It never throws. On errors config is null; errors are sorted by
 * their path (a JSON Pointer, '' for the whole config). Warnings name settings the schema allows that do nothing here.
 */
export function normalize(input) {
  let raw;
  try {
    // An object goes through JSON first, so it is read exactly as its JSON string would be: own keys only (nothing
    // inherited from a prototype), no undefined, no holes, no getters; a BigInt or a cycle is not JSON. The parsed
    // objects have no prototype, so a key the config lacks is never read from a polluted Object.prototype.
    const text = typeof input === 'string' ? input : JSON.stringify(input ?? null);
    raw = JSON.parse(text, (_, v) => (isObject(v) ? Object.assign(Object.create(null), v) : v)) ?? {};
  } catch {
    return { ok: false, config: null, errors: [{ path: '', message: 'is not valid JSON' }], warnings: [] };
  }
  const errors = [];
  check(SCHEMA, raw, '', errors);
  if (errors.length) {
    errors.sort((a, b) => byCodePoint(a.path, b.path));
    return { ok: false, config: null, errors, warnings: [] };
  }
  const kind = raw.kind ?? P.kind.default;
  const d = defaultsFor(kind);
  const config = { v: 1, kind };
  if (raw.source) config.source = normSource(raw.source);
  config.audio = raw.audio ?? d.audio;
  config.features = inOrder(raw.features ?? d.features, FEATURES);
  config.capabilities = inOrder(raw.capabilities ?? d.capabilities, CAPABILITIES);
  config.theme = raw.theme ?? P.theme.default;
  config.compact = raw.compact ?? P.compact.default;
  for (const k of ['width', 'height', 'title']) if (raw[k] !== undefined) config[k] = raw[k];
  config.site = raw.site ?? DEFAULT_SITE;
  config.bridge = raw.bridge ?? P.bridge.default;
  if (raw.pin) {
    config.pin = {};
    if (raw.pin.version !== undefined) config.pin.version = raw.pin.version;
    config.pin.sri = raw.pin.sri ?? P.pin.properties.sri.default;
  }

  const warnings = [];
  const s = raw.source;
  if (!s && kind !== 'studio') warnings.push('there is nothing to play: give text, cid, set or station');
  if (s && s.station === undefined && (s.n !== undefined || s.count !== undefined)) warnings.push('n and count apply only to a station');
  if (s && config.audio === 'inline' && !loneCid(config.source)) warnings.push('inline audio needs a stored program: a cid alone');
  if (config.bridge && kind !== 'studio') warnings.push('bridge applies only to the studio');
  return { ok: true, config, errors: [], warnings };
}

// ---- fromElement ----

const TAG_KIND = new Map([['beatscript-track', 'player'], ['beatscript-studio', 'studio']]);

/** As the player's sourceOf: the indentation of the markup is not part of the program. */
function dedent(t) {
  const lines = String(t || '').replace(/^\n+/, '').replace(/\s+$/, '').split('\n');
  // a loop, not Math.min(...lines): spreading a very long script overflows the call stack
  let ind = Infinity;
  for (const l of lines) if (l.trim()) ind = Math.min(ind, l.match(/^\s*/)[0].length);
  return lines.map((l) => l.slice(Number.isFinite(ind) ? ind : 0)).join('\n') + '\n';
}

const list = (v) => String(v).split(/[\s,]+/).filter(Boolean);
// a number only when it is written as an integer, so normalize can say what is wrong with anything else
const int = (v) => (/^\s*-?\d+\s*$/.test(v) ? Number(v) : v);

/**
 * The raw config an element describes (normalize it next): the JSON of its `config` attribute or of a child
 * <script type="application/json" class="beatscript-config">, then its single attributes over that, and the program
 * from a child <script type="text/beatscript"> when there is no `text` attribute. A <beatscript-studio> or
 * <beatscript-track> whose JSON gives no kind is a studio or a player. JSON that does not parse comes back as the string
 * itself, and JSON that is not an object as the value it is, so normalize reports it. el needs only getAttribute,
 * hasAttribute and querySelector (and tagName, for the kind).
 */
export function fromElement(el) {
  const attr = (name) => { const v = el.getAttribute(name); return v == null ? undefined : v; };
  // a boolean attribute: present is true, the value "false" is false
  const flag = (name) => (el.hasAttribute(name) ? attr(name) !== 'false' : undefined);

  const json = attr('config') ?? el.querySelector('script.beatscript-config[type="application/json"]')?.textContent;
  let raw = {};
  if (json != null) {
    try { raw = JSON.parse(json); } catch { return json; }
    if (!isObject(raw)) return raw;
  }
  if (raw.kind === undefined) {
    const kind = TAG_KIND.get(String(el.tagName || '').toLowerCase());
    if (kind) raw.kind = kind;
  }

  const over = {};
  const script = attr('text') === undefined ? el.querySelector('script[type="text/beatscript"], script[type="text/x-beatscript"]') : null;
  const text = attr('text') ?? (script ? dedent(script.textContent) : undefined);
  if (text !== undefined) over.text = text;
  for (const k of ['cid', 'station']) if (attr(k) !== undefined) over[k] = attr(k);
  for (const k of ['n', 'count']) if (attr(k) !== undefined) over[k] = int(attr(k));
  // a source that is not an object stays as it is, so normalize reports it
  if (Object.keys(over).length && (raw.source === undefined || isObject(raw.source))) raw.source = { ...raw.source, ...over };

  const audio = attr('audio') ?? attr('mode');
  if (audio !== undefined) raw.audio = audio;
  if (attr('features') !== undefined) raw.features = list(attr('features'));
  if (flag('highlight')) {
    const base = raw.features ?? defaultsFor(raw.kind).features;
    if (Array.isArray(base) && !base.includes('highlight')) raw.features = [...base, 'highlight'];
  }
  const caps = attr('capabilities') ?? attr('caps');
  if (caps !== undefined) raw.capabilities = list(caps);
  for (const k of ['theme', 'title', 'site']) if (attr(k) !== undefined) raw[k] = attr(k);
  for (const k of ['width', 'height']) if (attr(k) !== undefined) raw[k] = int(attr(k));
  for (const k of ['compact', 'bridge']) if (flag(k) !== undefined) raw[k] = flag(k);
  return raw;
}

// ---- toAttributes ----

const TAG = new Map([['player', 'beatscript-track'], ['studio', 'beatscript-studio']]);

const sameList = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);

/**
 * The element for a normalized config: {tag, attrs: [[name, value], ...], text}. The text goes in a child
 * <script type="text/beatscript">. Only what differs from the defaults is written, as attributes where there is one
 * (in the order cid, mode or audio, highlight, theme, compact, site; for the player, the attributes player 0.2.0 reads)
 * and the rest as the `config` attribute's compact JSON, last. The theme attribute is written only for light: player
 * 0.2.0 styles theme="light" and nothing else, so auto goes in the config, where the player resolves it from
 * prefers-color-scheme. fromElement reads it back to the same config. card and
 * audio are not elements: {tag: null, attrs: [], text: null}.
 */
export function toAttributes(config) {
  const tag = TAG.get(config.kind) || null;
  if (!tag) return { tag: null, attrs: [], text: null };
  const d = defaultsFor(config.kind);
  const s = config.source || {};
  const attrs = [];
  const rest = {};

  if (s.cid !== undefined) attrs.push(['cid', s.cid]);
  if (config.audio !== d.audio) {
    // player 0.2.0 reads mode="render"; it has no attribute for the server's audio
    const mode = config.kind === 'player' && (config.audio === 'live' || config.audio === 'render');
    attrs.push([mode ? 'mode' : 'audio', config.audio]);
  }
  const highlight = config.features.includes('highlight') && !d.features.includes('highlight');
  if (highlight) attrs.push(['highlight', '']);
  if (config.theme === 'light') attrs.push(['theme', config.theme]);
  if (config.compact) attrs.push(['compact', '']);
  if (config.site !== DEFAULT_SITE) attrs.push(['site', config.site]);

  const src = {};
  if (s.set) src.set = s.set;
  if (s.station !== undefined) {
    src.station = s.station;
    if (s.n !== N_DEFAULT) src.n = s.n;
    if (s.count !== COUNT_DEFAULT) src.count = s.count;
  }
  if (Object.keys(src).length) rest.source = src;
  // what fromElement makes of the features without a features key: the defaults, plus highlight if its attribute is set
  const implied = highlight ? inOrder([...d.features, 'highlight'], FEATURES) : d.features;
  if (!sameList(config.features, implied)) rest.features = config.features;
  if (!sameList(config.capabilities, d.capabilities)) rest.capabilities = config.capabilities;
  if (config.theme === 'auto') rest.theme = 'auto';
  for (const k of ['width', 'height', 'title']) if (config[k] !== undefined) rest[k] = config[k];
  if (config.bridge) rest.bridge = true;
  if (config.pin) rest.pin = config.pin;
  if (Object.keys(rest).length) attrs.push(['config', JSON.stringify(rest)]);

  return { tag, attrs, text: s.text ?? null };
}

// ---- artifactCapabilities ----

/** The upstream tools of the viewer's BeatScript connector that the studio calls. */
const CONNECTOR_TOOLS = ['share', 'render', 'queue'];
/**
 * The room topics the studio sends on, opened to the interact level (Contributor and above). An event topic the
 * declaration does not list is admin-only (room.d.ts), so without this only Editors could jam.
 */
const ROOM_TOPICS = { program: 'interact' };

/** The Claude Artifact tool's `capabilities` for a normalized config: {name: declaration}, in CAPABILITIES order. */
export function artifactCapabilities(config, { server = 'Beatscript' } = {}) {
  const out = {};
  for (const c of config.capabilities) {
    if (c === 'mcp') out.mcp = { servers: [{ server, tools: [...CONNECTOR_TOOLS] }] };
    else if (c === 'db') { out.db = {}; out.user = {}; } // the library is per user
    else if (c === 'downloads') out.downloads = true;
    else if (c === 'room') out.room = { topics: { ...ROOM_TOPICS } };
    else out[c] = {};
  }
  return out;
}

// ---- detectHost ----

/** The smallest WebAssembly module: a page whose CSP lacks 'wasm-unsafe-eval' refuses to compile even this. */
const EMPTY_MODULE = new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);

async function canCompile(wasm) {
  if (typeof wasm?.compile !== 'function') return false;
  try { await wasm.compile(EMPTY_MODULE); return true; } catch { return false; }
}

/** fn()'s value, or null when it throws, rejects, or takes longer than ms. */
function within(ms, fn) {
  return new Promise((done) => {
    const timer = setTimeout(() => done(null), ms);
    Promise.resolve().then(fn).then((v) => done(v ?? null), () => done(null)).finally(() => clearTimeout(timer));
  });
}

/**
 * What this page can do: {artifact, wasm, worklet, offline, clipboard, caps}. caps maps each wanted name to the
 * namespace claude.use(name) resolves, or null (not an artifact, not granted, or no answer within timeoutMs). Pass
 * want: [...CAPABILITIES, 'user'] for the user namespace too. use() asks no consent, so this is safe on page load.
 */
export async function detectHost(win = globalThis, { timeoutMs = 10000, want = CAPABILITIES } = {}) {
  const claude = win?.claude;
  const artifact = typeof claude?.use === 'function';
  const [wasm, caps] = await Promise.all([
    canCompile(win?.WebAssembly),
    Promise.all(want.map(async (name) => [name, artifact ? await within(timeoutMs, () => claude.use(name)) : null])),
  ]);
  return {
    artifact,
    wasm,
    worklet: typeof win?.AudioWorkletNode === 'function',
    offline: typeof win?.OfflineAudioContext === 'function',
    clipboard: typeof win?.navigator?.clipboard?.writeText === 'function',
    caps: Object.fromEntries(caps),
  };
}

// ---- resolve ----

export const NO_WASM = 'this page does not allow WebAssembly (\'wasm-unsafe-eval\'), so it cannot play here';
export const NO_WASM_STUDIO = 'this page does not allow WebAssembly (\'wasm-unsafe-eval\'), so the studio can show and save the text but cannot check, play or share it';

/**
 * Without the engine there is no check, no sound, no colour and no share link, and the studio makes no tools (so no
 * capability is used): what is left is the text, shown (the player's highlight, uncoloured), edited and saved as .btw.
 */
const ENGINELESS = { player: ['highlight'], studio: ['highlight', 'editor', 'download'] };
/** The features only <beatscript-studio> has (the player has no editor, mixer, knobs or compose). */
const STUDIO_ONLY = ['editor', 'mixer', 'knobs', 'compose'];
const NOT_BUILT = new Map([['mixer', 'there is no mixer yet'], ['knobs', 'there are no knobs yet']]);

/**
 * The graceful degrade: the config this host can honour, and a note for each thing it changed. A feature or capability
 * the host cannot do is dropped from the effective config (a plain download is kept, as a lesser download). Notes come
 * in a fixed order: the engine, the source, the features, the capabilities. Each message is one short plain sentence.
 * A player or studio on a page without WebAssembly gets the one engine note, and keeps only what needs no engine.
 */
export function resolve(config, host = {}) {
  const caps = host?.caps || {};
  const usable = (c) => config.capabilities.includes(c) && caps[c] != null;
  const out = { ...config };
  const notes = [];
  const note = (what, status, message) => { notes.push({ what, status, message }); };

  const engineless = Object.hasOwn(ENGINELESS, config.kind) ? ENGINELESS[config.kind] : null;
  if (engineless && host?.wasm === false) {
    // neither element plays the site's render: nothing is fetched from the site, so the audio setting changes nothing
    out.features = config.features.filter((f) => engineless.includes(f));
    out.capabilities = [];
    note('kind', 'unsupported', config.kind === 'studio' ? NO_WASM_STUDIO : NO_WASM);
    return { config: out, notes };
  }
  const src = config.source || {};
  if (config.kind === 'player' && src.station !== undefined) note('source', 'unsupported', 'the player cannot play a station');
  else if (config.kind === 'player' && src.set && src.set.length > 1) note('source', 'degraded', 'the player plays only the first item of the set');

  out.features = config.features.filter((f) => {
    if (STUDIO_ONLY.includes(f) && config.kind !== 'studio') {
      note(f, 'unsupported', `${f} needs kind studio`);
      return false;
    }
    if (NOT_BUILT.has(f)) { note(f, 'unsupported', NOT_BUILT.get(f)); return false; }
    if (f === 'compose' && !usable('sample')) {
      note(f, 'unsupported', 'compose needs Claude: the sample capability in a Claude artifact');
      return false;
    }
    if (f === 'download' && !usable('downloads')) note(f, 'degraded', 'download falls back to a plain browser download');
    return true;
  });
  out.capabilities = config.capabilities.filter((c) => {
    if (caps[c] != null) return true;
    note(c, 'unsupported', `the ${c} capability is not available here`);
    return false;
  });
  return { config: out, notes };
}

function deepFreeze(o) {
  if (o && typeof o === 'object') {
    for (const v of Object.values(o)) deepFreeze(v);
    Object.freeze(o);
  }
  return o;
}
