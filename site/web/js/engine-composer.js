/**
 * engine-composer.js: a module Worker that prepares tracks for the audio engine.
 *
 * Compiling a BeatScript program's nets into something playable (the net, its lanes, the measurements) takes tens of
 * milliseconds to a second or more. The audio thread must not stop for that, so a second instance of the engine
 * (`DjComposer`, from the same wasm module) does it here, encodes the result as bytes, and the worklet takes the
 * bytes in a few KB per render quantum (engine-worklet.js, `install`).
 *
 *   page -> worker   {type:'init', module, sampleRate}
 *                    {type:'prepare', id, deck, text}        text: the raw-nets share envelope, as JSON
 *                    {type:'wind', id, params}               a `wheel` call (params: its JSON): the Wheel's instrument wound
 *                                                            here (WheelWinder: a load, key, sequence or brightness), as the
 *                                                            booth winds off its audio lock; id 0 asks for no answer (a level)
 *                    {type:'wind-reset'}                     the instrument forgotten (the program stopped)
 *   worker -> page   {type:'ready'}   {type:'error', message}                       (init)
 *                    {type:'prepared', id, deck, bytes, ms}  bytes transferred
 *                    {type:'prepare-failed', id, deck, error}
 *                    {type:'wound', id, bytes}  {type:'wound', id, error}   bytes (transferred) for WheelVoice.install; empty:
 *                                                            nothing to install
 */

import * as glue from '../wasm/beatscript.js';

let composer = null;
let winder = null;

self.onmessage = (e) => {
    const m = e.data;
    if (m.type === 'init') {
        try {
            glue.initSync({ module: m.module });
            composer = new glue.DjComposer(m.sampleRate);
            self.postMessage({ type: 'ready' });
        } catch (err) {
            self.postMessage({ type: 'error', message: String((err && err.message) || err) });
        }
    } else if (m.type === 'prepare') {
        const t0 = performance.now();
        try {
            const bytes = composer.prepare_share(m.deck, m.text, '[]');
            self.postMessage({ type: 'prepared', id: m.id, deck: m.deck, bytes, ms: performance.now() - t0 }, [bytes.buffer]);
        } catch (err) {
            self.postMessage({ type: 'prepare-failed', id: m.id, deck: m.deck, error: String((err && err.message) || err) });
        }
    } else if (m.type === 'wind') {
        try {
            if (!winder) winder = new glue.WheelWinder();
            const bytes = winder.call(m.params);
            if (m.id) self.postMessage({ type: 'wound', id: m.id, bytes }, [bytes.buffer]);
        } catch (err) {
            if (m.id) self.postMessage({ type: 'wound', id: m.id, error: String((err && err.message) || err) });
        }
    } else if (m.type === 'wind-reset') {
        try { if (winder) winder.free(); } catch { /* gone */ }
        winder = null;
    }
};
