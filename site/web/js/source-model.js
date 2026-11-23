// Which sound source plays a program: the in-browser engine, the server's stream, or a render made in this browser and
// played as one audio file. Pure (app.js pickSource).

/** The sources a person can pick, and 'auto'. */
export const SOURCES = ['engine', 'stream', 'render'];

/** The remembered choice (localStorage) as a source or 'auto': anything else is auto. */
export function storedSource(v) {
    return SOURCES.includes(v) ? v : 'auto';
}

/**
 * 'engine' | 'stream' | 'render' | null.
 *   engine   the engine is loaded           stream  the program may go to the server (canStream)
 *   render   the page can render it here (the module is loaded, the program needs no asset files)
 *   pref     the person's choice: 'engine' | 'stream' | 'render' | anything else (auto)
 *   trigger  'eval' (an edit, Cmd+Enter) or 'load' (open, compose)
 *   loops    the program loops (its check's `loops`)
 *
 * Auto plays a loaded program on the stream (it starts sooner than a compile) and an edit on the engine (it lands on the
 * next bar). A program that LOOPS goes to the engine when the engine is there: the stream is a finite render of one pass,
 * so on the stream it would end, not start again (test/parity/loop.mjs).
 */
export function chooseSource({ engine, stream, render = false, pref, trigger, loops }) {
    if (pref === 'render') return render ? 'render' : null;
    if (pref === 'engine') return engine ? 'engine' : null;
    if (pref === 'stream') return stream ? 'stream' : null;
    if (trigger === 'load' && !(loops && engine)) return stream ? 'stream' : engine ? 'engine' : null;
    if (engine) return 'engine';
    return stream ? 'stream' : null;
}

/** What to say when a streamed render runs out: a looping program's stream is one pass of it. */
export function renderEndedMessage(loops) {
    return loops ? 'The render ended: the stream is one pass of a looping program. Play it on the engine to keep it looping.' : 'The render ended.';
}

/**
 * Whether the engine is stuttering enough to say "Audio stuttering? Try render.": `now` is what the browser and the
 * engine report so far, `was` the same a moment ago (both {underruns, inline, slow}: the context's underrun events
 * where the browser counts them, nets the worklet compiled on its own thread, the worklet's slow blocks). Two new
 * underruns, or any net compiled on the audio thread, is a stutter.
 */
export function stuttering(now, was) {
    const d = (k) => Math.max(0, (Number(now && now[k]) || 0) - (Number(was && was[k]) || 0));
    return d('underruns') >= 2 || d('inline') >= 1 || d('slow') >= 3;
}

export const STUTTER_HINT = 'Audio stuttering? Try render.';
