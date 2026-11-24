//! The Wheel in the browser: windwheel's linear tonewheel, the booth's instrument, as the page plays a program's `wheel`
//! lines. The instrument and its voice ([`Winder`], [`WheelCore`], [`Wound`]) live in `beatscript_api::wheel` (its `wheel`
//! feature), so the server's offline render (`beatscript render`) plays the very same code; here are only the module's
//! exports the composer Worker (`WheelWinder`) and the AudioWorklet (`WheelVoice`) call.

pub use beatscript_api::wheel::*;

// ---- the module's exports (the engine's kind: no `btw` name before them, as Dj and DjComposer have none) ----

use wasm_bindgen::prelude::*;


/// The Wheel's winding half (wheel.rs [`Winder`]), for the composer Worker: a `wheel` call's params in, the wound
/// instrument out as bytes for [`WheelVoice::install`] (empty: the call does not wind, it is the voice's alone).
#[wasm_bindgen]
pub struct WheelWinder {
    w: Winder,
}

#[wasm_bindgen]
impl WheelWinder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> WheelWinder {
        crate::init();
        WheelWinder { w: Winder::new() }
    }

    pub fn call(&mut self, params_json: &str) -> Result<Vec<u8>, JsError> {
        let a = Args::parse(params_json).map_err(|e| JsError::new(&e))?;
        match self.w.call(&a) {
            Ok(Some(w)) => Ok(w.encode()),
            Ok(None) => Ok(Vec::new()),
            Err(e) => Err(JsError::new(&e)),
        }
    }

    /// Forget the instrument (a new program starts with none).
    pub fn reset(&mut self) {
        self.w = Winder::new();
    }
}

impl Default for WheelWinder {
    fn default() -> Self {
        WheelWinder::new()
    }
}

/// Whether a `wheel` call's action winds (load, key, sequence, brightness: the Worker's) or is the voice's alone.
#[wasm_bindgen]
pub fn wheel_winds(action: &str) -> bool {
    winds(action)
}

/// The Wheel's voice (wheel.rs [`WheelCore`]), for the AudioWorklet: rendered after `Dj`, added to the master.
#[wasm_bindgen]
pub struct WheelVoice {
    c: WheelCore,
}

#[wasm_bindgen]
impl WheelVoice {
    #[wasm_bindgen(constructor)]
    pub fn new(sample_rate: f64) -> WheelVoice {
        WheelVoice { c: WheelCore::new(sample_rate) }
    }

    /// A wound instrument ([`WheelWinder::call`]'s bytes).
    pub fn install(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        let w = Wound::decode(bytes).map_err(|e| JsError::new(&e))?;
        self.c.install(&w);
        Ok(())
    }

    /// A `wheel` call the voice takes (play, pause, motor, level, sync ...), with the decks' tempos now (0: none).
    /// The answer as JSON; an error says why, as the booth would.
    pub fn call(&mut self, params_json: &str, bpm_a: f64, bpm_b: f64) -> Result<String, JsError> {
        let a = Args::parse(params_json).map_err(|e| JsError::new(&e))?;
        self.c.call(&a, |d| if d == "B" { bpm_b } else if d == "A" { bpm_a } else { 0.0 }).map_err(|e| JsError::new(&e))
    }

    /// A program's `master:` line as the decks get it: a master FX key (`lp-freq`, `hp-freq`, `crush-bits`) at its slider
    /// position 0..1, or `reverbWet` 0..1 ([`WheelCore::master`]).
    pub fn master(&mut self, name: &str, value: f64) -> Result<(), JsError> {
        self.c.master(name, value).map_err(|e| JsError::new(&e))
    }

    /// Add the next `left.len()` frames to `left` and `right`.
    pub fn render_add(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.c.render_add(left, right);
    }

    /// Whether it makes sound now (played, fading, or its DC blocker still settling): the worklet skips it otherwise.
    pub fn sounding(&self) -> bool {
        self.c.sounding()
    }

    pub fn status(&self) -> String {
        self.c.status()
    }
}

