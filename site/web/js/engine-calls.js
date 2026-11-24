/**
 * engine-calls.js: what a BeatScript program's control lines mean to the browser engine.
 *
 * `BeatscriptRun.service()` hands the page `{"op":"call","noun":"inserts","params":{...}}` for every control line that
 * fires: the same calls the desktop makes on its control-language registry. The desktop has the whole registry
 * (insert chains, moves, loops, auto-DJ, ...); the browser engine is djrust's public wasm facade (`Dj`), which has
 * a deck strip, a master FX unit and a few live ops. `mapCall` says what each call becomes there, as a list of
 * worklet ops, or why it is ignored. Pure (no audio, no DOM) so it is tested in Node.
 *
 * HONOURED
 *   mixer     {deck A|B, volume | gain | filter | low | mid | high | mute}  the deck strip, same 0..1 knobs
 *             {crossfader}  {master}                                        the mixer
 *   deck      {action: play | stop | pause, deck}                             the deck's transport
 *   inserts   target Master | A | B, effect filter   on / off, filter.sweep, filter.res -> that strip's insert chain (the
 *                                             booth's filter: djrust `live_op("inserts", ..)`, the same sweep and resonance
 *                                             curves; a ramp, `resonance 0.2 -> 0.4 over 8 bars`, is one `set` a pass)
 *             target Master, effect reverb    mix -> each deck strip's reverb wet (the master has no reverb here)
 *             target Master, effect crush     mix -> both decks' master FX crushers (crush-bits)
 *             target A | B, effect reverb     mix -> that strip's reverb wet
 *   fx        {action: set, param: drive, deck?, value}                       the deck's FX unit drive (the master chain's
 *             distortion: 0 is off, bit for bit); no deck is both decks (`master: drive 0.6`), A or B is that deck's
 *   vocoder   {action: set (or none) | swap | add | remove, instance?, carrier?, modulator?, mix, on, insert, ...}
 *             the engine's own vocoder (djrust `live_op("vocoder", ..)`): `carrier=A modulator=B` are decks (also
 *             `deck:A`, `synth`, `voice:robot-choir`), `insert=true` puts the result on the carrier's strip (its fader
 *             and the crossfader), else it plays on its own strip; a ramp (`mix=0 -> 1 over 8 bars`) is one `set` a pass
 *   moves     {action: arm, kind: vocoder_handoff, deck}  the vocoder's carrier and modulator swapped on the next
 *             phrase line (8 bars) of `deck`, as the desktop's move does
 *             {action: arm | toggle, kind: filter_exit | echo_out | loop_roll | bass_swap | phrase_sync (do=cut), deck A|B,
 *             bars?}  a {type:'move'} op: the worklet's MoveScheduler (below), a port of the booth's move scheduler
 *             (djhost perform.rs `Moves`), lands it on the deck's next bar or phrase line as the booth does
 *             {action: cancel, kind?, deck?} | {action: cancel_all}  the move let go as the booth lets it go
 *   wheel     {action: load | key | sequence | brightness | play | pause | toggle | motor | level | interval | sync |
 *             step | follow | pluck | trigger | status | presets, ...}  the Wheel: windwheel's tonewheel, the booth's own
 *             instrument (djweb src/wheel.rs). One op per call, `{type:'wheel', action, params}`: engine.js has a load,
 *             key, sequence or brightness wound in the composer Worker (the booth winds off the audio lock too) and the
 *             rest played by the worklet's voice, all in the order the program gave them. It plays after the decks into
 *             the master, through the master as the browser plays it (engine.js gives it the decks' `master:` filter,
 *             crush, reverb and level, set alike); status and presets ask, and play nothing
 *   (and `sound` commands, which are not calls: kit:NAME -> setKit, anything else -> setRoleVoice)
 *   A `sing` line is not a call here: on this host it is a part of its deck's net (djlive sing.rs, "In the nets"),
 *   so it is sung through the deck like any part and stops with the net.
 * IGNORED, with one console note per kind (and `check` warns about each where the program is written)
 *   inserts delay, filter mix, crush on a deck, delay time; the Wheel target
 *   vocal (a `sing` line that is timed or performed, `do vocal ...`), vocoder status / list / select / cycle_* / preset_*,
 *   a phrase_sync that steps a seed (do=seed, the default: a program's deck plays its own nets, not a seed; the booth's
 *   offline render does not play it either), a move on a deck other than A or B, moves status, loopsets, queue, autodj, td3,
 *   `deck load|sync`, mixer pfl / aux / headphone_*, and any other noun
 *   effects fire / release (a `macro` line, docs/beatscript/ext-control.md): noted by its id, `macro riser`, as the line says it; the
 *   offline render and `listen` play it, this engine plays none of the DJ catalogue yet
 * The server's render (`beatscript render`: the stream, `listen`, the stems) reads every call through a port of this file
 * (crates/beatscript-cli/src/browser.rs, held to it op for op by tests/browser_calls.rs) on the same djrust engine, so it plays
 * what this engine plays; what is IGNORED here it plays the booth's way.
 */

const clamp01 = (v) => (v < 0 ? 0 : v > 1 ? 1 : v);
const DECKS = ['A', 'B'];
/** The insert filter's knobs this engine plays (djrust `InsertParam` ids). */
const FILTER_KNOBS = ['filter.sweep', 'filter.res'];

/** The Wheel's actions (djhost control_registry `A_WHEEL`): the ones that wind the instrument, and the voice's. */
export const WHEEL_WINDS = new Set(['load', 'key', 'sequence', 'brightness']);
const WHEEL_ACTIONS = new Set([...WHEEL_WINDS, 'play', 'pause', 'toggle', 'motor', 'level', 'interval', 'sync', 'step', 'follow', 'pluck', 'trigger']);
const WHEEL_READS = new Set(['status', 'presets']);

/** The insert sweep knob's corners (mirror of djrust `fx::insert::sweep_corners`): {lp, hp} in Hz, or null. */
export function sweepCorners(v) {
    v = clamp01(v);
    const DEAD = 0.02;
    if (v < 0.5 - DEAD) {
        const t = (0.5 - DEAD - v) / (0.5 - DEAD);
        return { lp: 20000 * Math.pow(60 / 20000, t), hp: null };
    }
    if (v > 0.5 + DEAD) {
        const t = (v - 0.5 - DEAD) / (0.5 - DEAD);
        return { lp: null, hp: 20 * Math.pow(8000 / 20, t) };
    }
    return { lp: null, hp: null };
}

/** The deck strip's filter knob for a corner (inverse of djrust `fx::strip::filter_corners`). */
export function stripFilterKnob({ lp, hp }) {
    if (lp != null) {
        const t = Math.log(Math.max(200, Math.min(20000, lp)) / 20000) / Math.log(200 / 20000);
        return clamp01(0.45 - t * 0.45);
    }
    if (hp != null) {
        const t = Math.log(Math.max(20, Math.min(5000, hp)) / 20) / Math.log(5000 / 20);
        return clamp01(0.55 + t * 0.45);
    }
    return 0.5;
}

/** Master FX sliders (0..100, whole) for the corners (inverse of `MasterChain::lp_freq` / `hp_freq`). */
export function masterFilterSliders({ lp, hp }) {
    const slide = (hz, base, span) => Math.round(Math.max(0, Math.min(100, (100 * Math.log(hz / base)) / Math.log(span))));
    return {
        lp: lp != null ? slide(lp, 100, 200) : 100, // lp-freq 100 = open (20 kHz)
        hp: hp != null ? slide(hp, 20, 250) : 0, //    hp-freq 0 = open (20 Hz)
    };
}

/**
 * The master FX unit's slider `key` at `slider` (its own 0..100 units), as ops. Two things the engine's `masterFx` action
 * needs that are easy to get wrong (test/parity found both): its `value` is a POSITION, 0..1 of the slider's travel
 * (djrust `MasterFxKey::slider_value`), so a whole slider value above 1 was the end stop (a low-pass always open, a
 * high-pass or a crush always at the top); and the unit is the deck's own (beats' per-project master chain), so the
 * program's master is both decks' units set alike.
 */
const masterFx = (key, slider) => DECKS.map((deck) => ({ type: 'perform', deck, json: JSON.stringify({ op: 'masterFx', key, value: Math.round(slider) / 100 }) }));
const param = (deck, name, value) => ({ type: 'param', deck, name, value });
/** A djrust live op (`Dj.live_op(method, params)`): the params as canonical-ish JSON (keys sorted, so a ramp's unchanged pass dedupes). */
const live = (method, params) => ({ type: 'live', method, params: JSON.stringify(Object.fromEntries(Object.entries(params).sort(([a], [b]) => (a < b ? -1 : 1)))) });
const decksOf = (target) => (target === 'Master' ? DECKS : DECKS.includes(target) ? [target] : []);

/**
 * One registry call as worklet ops. Returns `{ops: [...]}` or `{ops: [], ignored: 'why'}`.
 * Ops: {type:'param', deck, name, value} | {type:'crossfader', value} | {type:'perform', deck, json} |
 *      {type:'play', deck, on} | {type:'live', method, params, phrase?: {deck, bars}} (a djrust live op, at once or on
 *      the next phrase line of `deck`) | {type:'wheel', action, params} (the Wheel: params the call's own, as JSON).
 */
export function mapCall(noun, params) {
    const p = params && typeof params === 'object' ? params : {};
    const num = (v) => (typeof v === 'number' ? v : typeof v === 'string' && v.trim() !== '' ? Number(v) : NaN);
    const ops = [];
    const skipped = [];
    switch (noun) {
    case 'mixer': {
        const deck = p.deck;
        if (p.crossfader !== undefined && Number.isFinite(num(p.crossfader))) ops.push({ type: 'crossfader', value: clamp01(num(p.crossfader)) });
        if (p.master !== undefined && Number.isFinite(num(p.master))) {
            ops.push({ type: 'perform', deck: 'A', json: JSON.stringify({ op: 'master', value: clamp01(num(p.master)) }) });
        }
        const strip = { volume: 'volume', gain: 'gain', filter: 'filter', low: 'eqLow', mid: 'eqMid', high: 'eqHigh' };
        if (DECKS.includes(deck)) {
            for (const [k, name] of Object.entries(strip)) {
                if (p[k] !== undefined && Number.isFinite(num(p[k]))) ops.push(param(deck, name, clamp01(num(p[k]))));
            }
            if (p.mute !== undefined) ops.push(param(deck, 'mute', p.mute === true || p.mute === 'true' || p.mute === 1 ? 1 : 0));
        } else if (deck !== undefined && Object.keys(strip).concat(['mute']).some((k) => p[k] !== undefined)) {
            skipped.push(`mixer deck ${deck}`);
        }
        for (const k of ['pfl', 'aux', 'headphone_mix', 'headphone_level']) if (p[k] !== undefined) skipped.push(`mixer ${k}`);
        break;
    }
    case 'deck': {
        const deck = p.deck;
        if (!DECKS.includes(deck)) { skipped.push('deck without A or B'); break; }
        if (p.action === 'play') ops.push({ type: 'play', deck, on: true });
        else if (p.action === 'stop' || p.action === 'pause') ops.push({ type: 'play', deck, on: false });
        else skipped.push(`deck ${p.action}`);
        break;
    }
    case 'inserts': {
        const target = p.target;
        const decks = decksOf(target);
        const effect = p.effect || (typeof p.param === 'string' ? p.param.split('.')[0] : undefined);
        if (!decks.length) { skipped.push(`inserts on ${target}`); break; }
        const on = p.action === 'on' ? p.on !== false : undefined;
        const value = num(p.value);
        if (effect === 'filter') {
            // the booth's own insert filter (djrust `fx::insert`, through the `inserts` live op): the same sweep corners, the same
            // resonance curve (Q 0.707 + res^2 * 9.3, none in the open middle) and the same glide, on the deck's strip or the master
            const t = target === 'Master' ? 'master' : target;
            if (p.action === 'on') ops.push(live('inserts', { target: t, effect: 'filter', on: on !== false }));
            else if (FILTER_KNOBS.includes(p.param) && Number.isFinite(value)) ops.push(live('inserts', { target: t, param: p.param, value: clamp01(value) }));
            else skipped.push(`inserts ${p.param || 'filter'}`);
        } else if (effect === 'reverb') {
            if (p.action === 'on' && on === false) for (const d of decks) ops.push(param(d, 'reverbWet', 0));
            else if (p.param === 'reverb.mix' && Number.isFinite(value)) for (const d of decks) ops.push(param(d, 'reverbWet', clamp01(value)));
            else if (p.action !== 'on') skipped.push(`inserts ${p.param || 'reverb'}`);
        } else if (effect === 'crush' && target === 'Master') {
            if (p.action === 'on' && on === false) ops.push(...masterFx('crush-bits', 0));
            else if (p.param === 'crush.mix' && Number.isFinite(value)) ops.push(...masterFx('crush-bits', clamp01(value) * 100));
            else if (p.action !== 'on') skipped.push(`inserts ${p.param || 'crush'}`);
        } else if (p.action === 'on' && on === false) {
            // switching an effect we do not have off is a no-op
        } else {
            skipped.push(`inserts ${effect || p.param || '?'}${target && target !== 'Master' ? ` on ${target}` : ''}`);
        }
        break;
    }
    case 'fx': {
        // the FX unit's drive: `deck A: drive 0.6`, and `master: drive 0.6` (the call with no deck: both decks', as the desktop's `fx` noun does)
        const value = num(p.value);
        const decks = p.deck === undefined ? DECKS : DECKS.includes(p.deck) ? [p.deck] : [];
        if (p.action === 'set' && p.param === 'drive' && decks.length) {
            if (Number.isFinite(value)) for (const deck of decks) ops.push({ type: 'perform', deck, json: JSON.stringify({ op: 'fx', param: 'drive', value: clamp01(value) }) });
        } else {
            skipped.push(p.action === 'set' && p.param ? `fx ${p.param}${p.deck && !decks.length ? ` on ${p.deck}` : ''}` : `fx ${p.action || '?'}`);
        }
        break;
    }
    case 'vocoder': {
        const action = p.action || 'set';
        const rest = {};
        for (const [k, v] of Object.entries(p)) if (k !== 'action') rest[k] = v;
        if (action === 'set' || action === 'on') {
            if (action === 'on' && rest.on === undefined) rest.on = true;
            if (Object.keys(rest).length) ops.push(live('vocoder', rest));
        } else if (action === 'swap') ops.push(live('vocoder', { ...rest, swap: true }));
        else if (action === 'add') ops.push(live('vocoder', { instance: rest.instance ?? 1, on: rest.on ?? true }));
        else if (action === 'remove') ops.push(live('vocoder', { instance: rest.instance ?? 1, remove: true }));
        else skipped.push(`vocoder ${action}`);
        break;
    }
    case 'effects': {
        // a `macro` line: named as the line names it (`macro riser`), so the page's "not played here" reads as the program
        if ((p.action === 'fire' || p.action === 'release') && typeof p.id === 'string' && p.id) skipped.push(`macro ${p.id}`);
        else skipped.push('effects');
        break;
    }
    case 'moves': {
        const deck = typeof p.deck === 'string' ? p.deck.trim().toUpperCase() : undefined;
        if (p.action === 'arm' && p.kind === 'vocoder_handoff') {
            ops.push({ ...live('vocoder', { swap: true }), phrase: { deck: DECKS.includes(deck) ? deck : 'A', bars: 8 } });
        } else if ((p.action === 'arm' || p.action === 'toggle') && SCHEDULED_MOVES.includes(p.kind)) {
            if (!DECKS.includes(deck)) skipped.push(`moves on deck ${p.deck}`);
            else if (p.kind === 'phrase_sync' && phraseAction(p.do) !== 'cut') skipped.push('moves phrase_sync seed');
            else ops.push({ type: 'move', action: p.action, kind: p.kind, deck, bars: moveBars(p.kind, num(p.bars)), do: p.kind === 'phrase_sync' ? 'cut' : undefined });
        } else if (p.action === 'cancel' && (p.kind === undefined || MOVE_KINDS.includes(p.kind))) {
            ops.push({ type: 'move', action: 'cancel', kind: p.kind, deck });
        } else if (p.action === 'cancel_all') {
            ops.push({ type: 'move', action: 'cancel_all' });
        } else skipped.push(`moves ${p.action || '?'}${p.kind ? ` ${p.kind}` : ''}`);
        break;
    }
    case 'wheel': {
        const action = typeof p.action === 'string' ? p.action : 'status';
        if (WHEEL_ACTIONS.has(action)) ops.push({ type: 'wheel', action, params: JSON.stringify({ ...p, action }) });
        else if (!WHEEL_READS.has(action)) skipped.push(`wheel ${action}`);
        break;
    }
    default:
        skipped.push(noun);
    }
    return skipped.length ? { ops, ignored: skipped.join(', ') } : { ops };
}

/**
 * A `sound` command ({deck, role, option}) as a worklet op: `kit:NAME` is a drum kit, anything else the role's voice
 * (`synth:reese`, `inst:NAME`, a voice name).
 */
export function mapSound(deck, role, option) {
    const kit = typeof option === 'string' && option.startsWith('kit:') ? option.slice(4) : null;
    const json = kit !== null ? { op: 'setKit', deck, kit } : { op: 'setRoleVoice', deck, role, voice: option };
    return { type: 'perform', deck, json: JSON.stringify(json) };
}

// ---- the moves (djhost perform.rs, `Moves`) ----------------------------------------------------------------------------

/** Every move the language has (djlive compile.rs MOVES). */
export const MOVE_KINDS = ['filter_exit', 'echo_out', 'loop_roll', 'bass_swap', 'phrase_sync', 'vocoder_handoff'];
/** The moves the worklet's MoveScheduler times (vocoder_handoff is a phrased live op of its own, above). */
export const SCHEDULED_MOVES = ['filter_exit', 'echo_out', 'loop_roll', 'bass_swap', 'phrase_sync'];
/** How far into the high-pass the filter exit rides (perform.rs FILTER_EXIT_AMOUNT). */
export const FILTER_EXIT_AMOUNT = 0.9;
/** The wash's dry fade on an echo out, in beats (perform.rs ECHO_CUT_BEATS). */
export const ECHO_CUT_BEATS = 0.25;
/** A loop roll's stages on a deck that loops (a net deck, which every deck of a program is), in beats (perform.rs ROLL_BEATS). */
export const ROLL_BEATS = [4, 2, 1, 0.5];
const TICKS_PER_BAR = 16;
const BEATS_PER_BAR = 4;

/** What a phrase sync does on its line (perform.rs `PhraseAction::parse`; no `do`, or one it does not know, is a seed). */
export function phraseAction(v) {
    const s = typeof v === 'string' ? v.trim().toLowerCase() : '';
    return ['cut', 'xfade', 'crossfader', 'transition'].includes(s) ? 'cut' : 'seed';
}

/** A move's `bars` as `Moves::arm` takes it: none or 0 is the move's own default, at most 64. */
export function moveBars(kind, bars) {
    const def = { bass_swap: 8, vocoder_handoff: 8, phrase_sync: 16, filter_exit: 4, echo_out: 4, loop_roll: 1 }[kind] || 4;
    const b = Number.isFinite(bars) ? Math.max(0, Math.floor(bars)) : 0;
    return Math.min(64, b === 0 ? def : b);
}

/**
 * The booth's move scheduler (djhost perform.rs `Moves`) on the browser engine: a move armed now lands on the next bar line
 * (filter_exit, echo_out, loop_roll) or phrase line (bass_swap, phrase_sync) of the deck that times it, and runs its schedule
 * over the engine's primitives, as the booth's does:
 *
 *   bass_swap    at arm: the incoming deck's low EQ cut; on the outgoing deck's phrase line: the lows swapped
 *   phrase_sync  (do=cut) on the phrase line of the deck playing out: the crossfader cut over to the deck
 *   filter_exit  on the bar line: the `filter` trick rides a high-pass over `bars`; `bars` later the deck is muted, the filter
 *                back open behind it
 *   echo_out     on the bar line: the `wash` trick (the dry gone in a 16th, the room ringing); `bars` later the deck muted, the
 *                wash off behind it
 *   loop_roll    on the bar line: a 4-beat loop, halved every `bars` bars on the move's own clock (4 -> 2 -> 1 -> 1/2 beat),
 *                released on the line after the last (the drop)
 *
 * A stopped deck acts at once, as a stopped grid does in the booth. One of each kind a deck (a bass swap: one at all): arming
 * again cancels the one in flight. The program puts a muted strip back itself (the runtime's `mixer {mute: false}` on the
 * first phrase line after the move), as it does in the booth.
 *
 * Lines are the worklet's, as for the vocoder handoff: a deck's place in its net in ticks (16 to the bar), a line crossed when
 * the place passes a multiple of the period or wraps (a new net lands on a bar line). Pure: everything goes through `host`:
 *   position(deck) -> ticks | null (not playing)    bpm(deck) -> number       eqLow(deck) -> 0..1
 *   setEqLow(deck, v)   setMute(deck, on)   trick(deck, kind, on, args?)   loopBeats(deck, beats)   loopHalve(deck)
 *   exitLoop(deck)      crossfadeTo(deck)
 */
export class MoveScheduler {
    constructor(host) {
        this.host = host;
        this.active = [];
        this.nextId = 0;
        /** The last word on each move, by `kind:deck` (`kind:*` for a bass swap), as the booth's `last`. */
        this.last = {};
    }

    get busy() { return this.active.length > 0; }

    other(deck) { return deck === 'A' ? 'B' : 'A'; }

    plays(deck) {
        const pos = this.host.position(deck);
        return pos !== null && pos !== undefined && this.host.bpm(deck) > 0;
    }

    /** perform.rs `pick_grid`: `first` if it plays, else `then` if it does, else null (at once). */
    pickGrid(first, then) {
        if (this.plays(first)) return first;
        return then && this.plays(then) ? then : null;
    }

    oneAtATime(kind) { return kind === 'bass_swap'; }

    note(kind, deck, text) { this.last[`${kind}:${this.oneAtATime(kind) ? '*' : deck}`] = text; }

    /** A {type:'move'} op from mapCall. */
    apply(op) {
        if (op.action === 'cancel_all') return this.cancelAll();
        if (op.action === 'cancel') return this.cancelKind(op.kind, op.deck);
        if (op.action === 'toggle' && this.find(op.kind, op.deck)) return this.cancelKind(op.kind, op.deck);
        if (op.action === 'arm' || op.action === 'toggle') return this.arm(op.kind, op.deck, op.bars);
        return false;
    }

    find(kind, deck) {
        return this.active.find((m) => m.kind === kind && (this.oneAtATime(kind) || m.deck === deck)) || null;
    }

    /** `Moves::arm`. Returns the move's id. */
    arm(kind, deck, bars) {
        if (!SCHEDULED_MOVES.includes(kind) || (deck !== 'A' && deck !== 'B')) return null;
        bars = moveBars(kind, bars);
        for (const m of this.active.filter((x) => x.kind === kind && (this.oneAtATime(kind) || x.deck === deck))) this.cancel(m.id);
        const out = this.other(deck);
        const m = { id: ++this.nextId, kind, deck, bars, phase: 'armed', grid: null, ticks: TICKS_PER_BAR, last: null, left: 1, savedLow: 0.5, stage: 0, elapsed: 0 };
        if (kind === 'bass_swap') {
            // the outgoing deck holds the phrase: it is the one playing out
            m.savedLow = this.host.eqLow(deck) ?? 0.5;
            m.grid = this.pickGrid(out, deck);
            m.ticks = bars * TICKS_PER_BAR;
        } else if (kind === 'phrase_sync') {
            m.grid = this.pickGrid(out, deck);
            m.ticks = bars * TICKS_PER_BAR;
        } else {
            m.grid = this.pickGrid(deck, null);
        }
        m.last = m.grid ? this.host.position(m.grid) : null;
        // what happens at once
        if (kind === 'bass_swap' && m.savedLow > 0) this.host.setEqLow(deck, 0);
        this.active.push(m);
        this.note(kind, deck, m.grid ? `armed: on the ${m.ticks === TICKS_PER_BAR ? 'next bar' : `next ${bars}-bar phrase`} line of ${m.grid}` : 'armed: at once (not playing)');
        return m.id;
    }

    /** `Moves::cancel`: one waiting is dropped (a bass swap gives the incoming lows back), one running let go. */
    cancel(id) {
        const i = this.active.findIndex((m) => m.id === id);
        if (i < 0) return false;
        const [m] = this.active.splice(i, 1);
        if (m.kind === 'bass_swap' && m.phase === 'armed') this.host.setEqLow(m.deck, m.savedLow);
        else if (m.kind === 'filter_exit' && m.phase === 'running') this.host.trick(m.deck, 'filter', false);
        else if (m.kind === 'echo_out' && m.phase === 'running') this.host.trick(m.deck, 'wash', false);
        else if (m.kind === 'loop_roll' && m.phase === 'running') this.host.exitLoop(m.deck);
        this.note(m.kind, m.deck, 'cancelled');
        return true;
    }

    /** The move of `kind` on `deck` (any deck for a bass swap) cancelled; no kind: every move on `deck`, or every move. */
    cancelKind(kind, deck) {
        const ids = this.active.filter((m) => (kind === undefined || m.kind === kind) && (deck === undefined || (kind && this.oneAtATime(kind)) || m.deck === deck)).map((m) => m.id);
        for (const id of ids) this.cancel(id);
        return ids.length > 0;
    }

    cancelAll() {
        for (const m of this.active.slice()) this.cancel(m.id);
        return true;
    }

    /**
     * Whether `m`'s next line has come: `m.left` lines of `m.ticks` on its grid deck. A deck that stopped (or a move with no
     * grid) is there at once, as a stopped grid is in the booth.
     */
    lineCome(m) {
        if (!m.grid) return true;
        const pos = this.host.position(m.grid);
        if (pos === null || pos === undefined) return true;
        const last = m.last === null || m.last === undefined ? pos : m.last;
        if (pos < last || Math.floor(pos / m.ticks + 1e-9) !== Math.floor(last / m.ticks + 1e-9)) m.left--;
        m.last = pos;
        return m.left <= 0;
    }

    /** One pass of `dt` seconds of audio: every move whose line has come acts (`Moves::service`). */
    service(dt) {
        for (const m of this.active.slice()) {
            const word = this.step(m, dt);
            if (word !== null) {
                this.active = this.active.filter((x) => x !== m);
                this.note(m.kind, m.deck, word);
            }
        }
    }

    /** Advance one move; the last word when it is over, else null. */
    step(m, dt) {
        const h = this.host;
        if (m.kind === 'loop_roll' && m.phase === 'running') {
            // a roll keeps its own beats: the looping deck's place wraps
            if (this.plays(m.deck)) m.elapsed += (dt * h.bpm(m.deck)) / 60;
            const stepBeats = m.bars * BEATS_PER_BAR;
            if (m.elapsed < (m.stage + 1) * stepBeats - 1e-6) return null;
            m.stage++;
            if (m.stage < ROLL_BEATS.length) { h.loopHalve(m.deck); return null; }
            h.exitLoop(m.deck);
            return `${m.deck} roll released on the drop`;
        }
        const stopped = !m.grid || !this.plays(m.grid);
        if (!this.lineCome(m)) return null;
        const deck = m.deck;
        switch (m.kind) {
        case 'bass_swap': {
            const out = this.other(deck);
            const give = m.savedLow > 0 ? m.savedLow : Math.max(h.eqLow(out) ?? 0.5, 0.5);
            h.setEqLow(deck, give);
            h.setEqLow(out, 0);
            return `lows swapped ${out} → ${deck}`;
        }
        case 'phrase_sync':
            h.crossfadeTo(deck);
            return `crossfader cut to ${deck}`;
        case 'filter_exit':
        case 'echo_out': {
            const trick = m.kind === 'filter_exit' ? 'filter' : 'wash';
            if (m.phase === 'armed') {
                h.trick(deck, trick, true, m.kind === 'filter_exit' ? { amount: FILTER_EXIT_AMOUNT, bars: m.bars } : { beats: ECHO_CUT_BEATS, amount: 1 });
                if (stopped) {
                    // a stopped deck has no downbeat to wait for, and nothing rings
                    h.setMute(deck, true);
                    h.trick(deck, trick, false);
                    return `${deck} cut at once (stopped)`;
                }
                m.phase = 'running';
                m.grid = deck;
                m.ticks = TICKS_PER_BAR;
                m.left = m.bars;
                return null;
            }
            h.setMute(deck, true);
            h.trick(deck, trick, false);
            return m.kind === 'filter_exit' ? `${deck} cut` : `${deck} tail over, channel cut`;
        }
        case 'loop_roll':
            if (stopped) return `${deck} is not playing: nothing to roll`;
            h.loopBeats(deck, ROLL_BEATS[0]);
            m.phase = 'running';
            m.stage = 0;
            m.elapsed = 0;
            return null;
        }
        return 'not a move';
    }
}
