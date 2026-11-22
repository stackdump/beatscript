//! Recovery by analysis by synthesis (docs/ROUNDTRIP.md, sections 1 and 3).
//!
//! 1. Grid: tempo and bar count from the length of a `btwd` render (`bars * 240 / bpm` seconds), the tempo
//!    family settled by where the onsets fall.
//! 2. Seed: orthogonal matching pursuit over rendered single-event templates, in a phase-blind band-power
//!    feature space (`spec`), all gains refitted together. This proposes; it does not decide.
//! 3. Climb: every move (remove an event, move a bass note, add a proposal from what the current render leaves
//!    unexplained) is scored by really rendering it with `btwd`.
//! 4. Velocity: scan a part's velocity against each hit's own window to read per-hit velocities, then find the
//!    `vel=lo..hi` whose compiled net carries exactly those velocities.
//! 5. Score: L0 (samples) and, with a reference, L1/L2 (`same_as`).
//!
//! The judge is the spectral distance while searching (noise voices draw different noise per hit, so samples
//! cannot rank partial programs) and the samples at the end (the real proof).

use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

use crate::btwd::{self, Btwd};
use crate::model::{note_name, triads, State, Vel, DRUMS, STEPS};
use crate::score::{self, L0};
use crate::spec::{band_weights, spec_of, Spec, HOP, NFFT};
use crate::vel;
use crate::wav::Pcm;

#[derive(Clone, Debug)]
pub struct Options {
    pub bass_voices: Vec<String>,
    pub pad_voices: Vec<String>,
    /// MIDI range tried for the bass.
    pub bass_notes: (u8, u8),
    /// Concurrent `btwd render` processes.
    pub jobs: usize,
    pub verbose: bool,
}

impl Default for Options {
    fn default() -> Options {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect();
        Options {
            bass_voices: s(&["sub-bass", "bass", "fm-bass"]),
            pad_voices: s(&["warm-pad", "pad", "strings"]),
            bass_notes: (28, 64),
            jobs: 4,
            verbose: false,
        }
    }
}

#[derive(Debug)]
pub struct Report {
    pub text: String,
    pub l0: L0,
    pub same_as: Option<(bool, Vec<String>)>,
    pub log: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Drum(String),
    Bass { inst: String, midi: u8 },
    Chord { chord: String, inst: String },
}

/// A rendered single event, as band powers.
struct Template {
    key: Key,
    /// nf frames x bands, starting at analysis frame `k0` of the template's own render.
    feat: Vec<f32>,
    nf: usize,
    k0: i64,
    /// Sample position the template's event had in its own render.
    p0: i64,
    norm2: f64,
}

#[derive(Clone, Copy, Debug)]
struct Grid {
    bpm: f64,
    bars: u32,
}

impl Grid {
    fn bar_frames(&self) -> f64 {
        240.0 / self.bpm * 44100.0
    }
    fn step_frames(&self) -> f64 {
        self.bar_frames() / STEPS as f64
    }
    /// The frame an event of a step starts on: the renderer places an event on the next whole frame.
    fn pos(&self, bar: u32, step: usize) -> usize {
        (((bar as usize * STEPS + step) as f64) * self.step_frames()).ceil() as usize
    }
}

/// (bpm, bars) a render of `frames` frames could be: whole or half BPM in 60..=200, nearest 120 first.
pub fn grid_candidates(frames: usize, sr: u32) -> Vec<(f64, u32)> {
    let secs = frames as f64 / sr as f64;
    let mut v = Vec::new();
    for bars in 1..=64u32 {
        let bpm = 240.0 * bars as f64 / secs;
        let r = (bpm * 2.0).round() / 2.0;
        // A render's length is bars * 240 / bpm seconds to the frame: within 3 frames, not 0.01 bpm.
        if (bpm - r).abs() < bpm * 3.0 / frames as f64 && (60.0..=200.0).contains(&r) {
            v.push((r, bars));
        }
    }
    v.sort_by(|a, b| (a.0 - 120.0).abs().partial_cmp(&(b.0 - 120.0).abs()).unwrap());
    v
}

fn par_map<T: Send, R: Send>(items: Vec<T>, jobs: usize, f: impl Fn(T) -> R + Sync) -> Vec<R> {
    let n = items.len();
    let queue = Mutex::new(items.into_iter().enumerate().collect::<VecDeque<_>>());
    let out: Mutex<Vec<Option<R>>> = Mutex::new((0..n).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..jobs.max(1).min(n.max(1)) {
            s.spawn(|| loop {
                let job = queue.lock().unwrap().pop_front();
                let Some((i, item)) = job else { break };
                let r = f(item);
                out.lock().unwrap()[i] = Some(r);
            });
        }
    });
    out.into_inner().unwrap().into_iter().map(|x| x.unwrap()).collect()
}

/// Solve the (small, symmetric) normal equations by Gaussian elimination with partial pivoting.
fn solve(gram: &[Vec<f64>], rhs: &[f64]) -> Vec<f64> {
    let n = rhs.len();
    let mut m: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            let mut row = gram[i].clone();
            row[i] += 1e-9 * gram[i][i].abs().max(1e-30); // a hair of ridge keeps duplicates solvable
            row.push(rhs[i]);
            row
        })
        .collect();
    for i in 0..n {
        let p = (i..n).max_by(|a, b| m[*a][i].abs().partial_cmp(&m[*b][i].abs()).unwrap()).unwrap();
        m.swap(i, p);
        let d = m[i][i];
        if d.abs() < 1e-300 {
            continue;
        }
        for r in 0..n {
            if r != i {
                let f = m[r][i] / d;
                if f != 0.0 {
                    for c in i..=n {
                        let x = m[i][c];
                        m[r][c] -= f * x;
                    }
                }
            }
        }
    }
    (0..n).map(|i| if m[i][i].abs() < 1e-300 { 0.0 } else { m[i][n] / m[i][i] }).collect()
}

struct Recoverer<'a> {
    btwd: &'a Btwd,
    opts: &'a Options,
    target_pcm: &'a Pcm,
    target: Spec,
    w: Vec<f32>,
    target_energy: f64,
    log: Vec<String>,
}

impl<'a> Recoverer<'a> {
    fn say(&mut self, s: String) {
        if self.opts.verbose {
            eprintln!("{s}");
        }
        self.log.push(s);
    }

    /// Weighted inner product of a template with a spectrogram-shaped buffer, the template placed `delta` frames on.
    fn wdot_placed(&self, buf: &[f32], t: &Template, delta: i64) -> f64 {
        let bands = self.target.bands;
        let mut c = 0f64;
        for j in 0..t.nf {
            let k = t.k0 + j as i64 + delta;
            if k < 0 || k as usize >= self.target.frames {
                continue;
            }
            let (a, b) = (&buf[k as usize * bands..(k as usize + 1) * bands], &t.feat[j * bands..(j + 1) * bands]);
            for i in 0..bands {
                c += (a[i] * b[i] * self.w[i]) as f64;
            }
        }
        c
    }

    fn delta(&self, grid: Grid, t: &Template, bar: u32, step: usize) -> i64 {
        ((grid.pos(bar, step) as i64 - t.p0) as f64 / HOP as f64).round() as i64
    }

    /// Render a state (every bar): the real thing, for the end.
    fn eval(&self, st: &State) -> Result<Pcm, String> {
        if st.is_empty() {
            return Ok(Pcm { sr: 44100, samples: Vec::new() });
        }
        self.btwd.render(&st.text())
    }

    fn eval_n(&self, st: &State, bars: u32) -> Result<Pcm, String> {
        let mut probe = st.clone();
        probe.bars = bars.min(st.bars).max(1);
        self.eval(&probe)
    }

    /// Spectral distance between the first `bars` bars of a render of `st` and of the target.
    fn eval_probe(&self, st: &State, bars: u32) -> Result<f64, String> {
        let pcm = self.eval_n(st, bars)?;
        let cand = spec_of(&pcm.samples, 44100.0);
        let upto = ((bars.min(st.bars).max(1) as f64 * 240.0 / st.bpm * 44100.0) as usize).saturating_sub(NFFT) / HOP;
        Ok(self.spec_error(&cand, 0, upto.min(self.target.frames)))
    }

    /// Weighted squared band-power error over target frames [from, to); frames the candidate lacks count as silence.
    fn spec_error(&self, cand: &Spec, from: usize, to: usize) -> f64 {
        let bands = self.target.bands;
        let mut e = 0f64;
        for f in from..to {
            let t = self.target.row(f);
            let c = if f < cand.frames { Some(cand.row(f)) } else { None };
            for b in 0..bands {
                let d = t[b] - c.map_or(0.0, |r| r[b]);
                e += (d * d * self.w[b]) as f64;
            }
        }
        e
    }

    /// Squared sample error of `pcm` against the target over interleaved samples [from, to).
    fn wave_error(&self, pcm: &Pcm, from: usize, to: usize) -> f64 {
        let t = &self.target_pcm.samples;
        (from..to.min(t.len())).map(|i| (t[i] as f64 - pcm.samples.get(i).copied().unwrap_or(0) as f64).powi(2)).sum()
    }

    /// Build one template: the event at slot 0 of every bar, 2 bars rendered, bar 1's frames kept (so the
    /// previous bar's tail is in it, as it is in the real mix).
    fn template(&self, key: Key, st: State, win_steps: f64) -> Option<Template> {
        let pcm = self.btwd.render(&st.text()).ok()?;
        let s = spec_of(&pcm.samples, 44100.0);
        let bar = 240.0 / st.bpm * 44100.0;
        let p0 = bar.ceil() as i64;
        let k0 = ((p0 - NFFT as i64) / HOP as i64).max(0);
        let win = (bar / STEPS as f64 * win_steps) as i64;
        let k_end = ((p0 + win) / HOP as i64).min(s.frames as i64 - 1);
        if k_end < k0 {
            return None;
        }
        let nf = (k_end - k0 + 1) as usize;
        let feat: Vec<f32> = s.data[k0 as usize * s.bands..(k0 as usize + nf) * s.bands].to_vec();
        let norm2: f64 = (0..nf).map(|j| (0..s.bands).map(|b| (feat[j * s.bands + b].powi(2) * self.w[b]) as f64).sum::<f64>()).sum();
        if norm2 <= 0.0 {
            return None;
        }
        Some(Template { key, feat, nf, k0, p0, norm2 })
    }

    fn bank(&self, items: Vec<(Key, State)>, win_steps: f64) -> Vec<Template> {
        par_map(items, self.opts.jobs, |(k, s)| self.template(k, s, win_steps)).into_iter().flatten().collect()
    }

    /// Predicted reduction of the weighted error from adding `t` at `step` on every bar, against residual `r`.
    fn predict(&self, grid: Grid, r: &[f32], t: &Template, step: usize) -> f64 {
        let mut c = 0f64;
        for b in 0..grid.bars {
            c += self.wdot_placed(r, t, self.delta(grid, t, b, step));
        }
        if c <= 0.0 {
            0.0
        } else {
            c * c / (grid.bars as f64 * t.norm2)
        }
    }

    /// The pursuit's column: the template placed at `step` of every bar.
    fn column(&self, grid: Grid, t: &Template, step: usize) -> Vec<f32> {
        let bands = self.target.bands;
        let mut v = vec![0f32; self.target.frames * bands];
        for b in 0..grid.bars {
            let d = self.delta(grid, t, b, step);
            for j in 0..t.nf {
                let k = t.k0 + j as i64 + d;
                if k < 0 || k as usize >= self.target.frames {
                    continue;
                }
                for i in 0..bands {
                    v[k as usize * bands + i] += t.feat[j * bands + i];
                }
            }
        }
        v
    }

    fn gdot(&self, a: &[f32], b: &[f32]) -> f64 {
        let bands = self.target.bands;
        let mut c = 0f64;
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            c += (x * y * self.w[i % bands]) as f64;
        }
        c
    }

    /// Orthogonal matching pursuit: pick the (template, step) that explains the most of what is left, refit all
    /// gains together (least squares, no negative gains), repeat. Joint refitting is the point: a kick and a bass
    /// share the low end, so judging one hit at a time against a residual that still holds the other misleads.
    fn omp(&mut self, grid: Grid, bank: &[Template], allow: &dyn Fn(&Key, usize) -> bool, max_sel: usize, floor: f64) -> (Vec<(usize, usize, f64)>, f64) {
        struct Col {
            ti: usize,
            step: usize,
            v: Vec<f32>,
        }
        let mut sel: Vec<Col> = Vec::new();
        let mut gram: Vec<Vec<f64>> = Vec::new();
        let mut rhs: Vec<f64> = Vec::new();
        let mut gains: Vec<f64> = Vec::new();
        let mut banned: HashSet<(usize, usize)> = HashSet::new();
        let mut r = self.target.data.clone();
        let mut r_energy = self.target_energy;
        let mut guard = 0;
        while sel.len() < max_sel && guard < max_sel * 3 {
            guard += 1;
            let mut best: Option<(f64, usize, usize)> = None;
            for (ti, t) in bank.iter().enumerate() {
                for step in 0..STEPS {
                    if !allow(&t.key, step) || banned.contains(&(ti, step)) || sel.iter().any(|c| c.ti == ti && c.step == step) {
                        continue;
                    }
                    let p = self.predict(grid, &r, t, step);
                    if best.map_or(true, |b| p > b.0) {
                        best = Some((p, ti, step));
                    }
                }
            }
            let Some((p, ti, step)) = best else { break };
            if p < floor * self.target_energy {
                break;
            }
            let v = self.column(grid, &bank[ti], step);
            let mut row: Vec<f64> = sel.iter().map(|c| self.gdot(&c.v, &v)).collect();
            let selfdot = self.gdot(&v, &v);
            for (j, x) in row.iter().enumerate() {
                gram[j].push(*x);
            }
            row.push(selfdot);
            gram.push(row);
            rhs.push(self.gdot(&self.target.data, &v));
            sel.push(Col { ti, step, v });
            // A negative gain means the column explains nothing by itself: drop the worst and refit.
            loop {
                gains = solve(&gram, &rhs);
                let worst = (0..sel.len()).min_by(|a, b| gains[*a].partial_cmp(&gains[*b]).unwrap());
                match worst {
                    Some(w) if gains[w] < 0.0 => {
                        banned.insert((sel[w].ti, sel[w].step));
                        sel.remove(w);
                        gram.remove(w);
                        for row in gram.iter_mut() {
                            row.remove(w);
                        }
                        rhs.remove(w);
                    }
                    _ => break,
                }
            }
            r.copy_from_slice(&self.target.data);
            for (c, g) in sel.iter().zip(&gains) {
                let g = *g as f32;
                for (x, y) in r.iter_mut().zip(&c.v) {
                    *x -= g * y;
                }
            }
            r_energy = r.iter().enumerate().map(|(i, x)| (x * x * self.w[i % self.target.bands]) as f64).sum();
        }
        (sel.iter().zip(&gains).map(|(c, g)| (c.ti, c.step, *g)).collect(), r_energy)
    }

    /// Onset strength: the rise of a short amplitude envelope over the mean of the 8 hops before it (hop 128 frames).
    fn onset_flux(&self) -> Vec<f32> {
        let hop = 128;
        let s = &self.target_pcm.samples;
        let n = s.len() / 2 / hop;
        let env: Vec<f32> = (0..n).map(|k| s[k * hop * 2..(k + 1) * hop * 2].iter().map(|x| (*x as f32).abs()).sum::<f32>() / (hop * 2) as f32).collect();
        (0..n)
            .map(|k| {
                let lo = k.saturating_sub(8);
                let prev = if k > lo { env[lo..k].iter().sum::<f32>() / (k - lo) as f32 } else { 0.0 };
                (env[k] - prev).max(0.0)
            })
            .collect()
    }

    /// Choose the tempo and bar count that fit the length. They come in a family (62 x 4, 93 x 6, 124 x 8, ...,
    /// bars = bpm / 15.5 for this length), so the length cannot say which. The onsets can: the right lattice of
    /// sixteenth steps catches onset mass the same lattice shifted by half a step does not. Ties go to the
    /// coarsest lattice (fewest bars), so a track that only ever uses every other step reads at the slower tempo.
    fn pick_grid(&mut self, frames: usize) -> Result<Grid, String> {
        let cands = grid_candidates(frames, 44100);
        if cands.is_empty() {
            return Err("the length is no whole bar count at a tempo of 60 to 200 (not a btwd render?)".into());
        }
        if cands.len() == 1 {
            return Ok(Grid { bpm: cands[0].0, bars: cands[0].1 });
        }
        let flux = self.onset_flux();
        let hop = 128.0;
        let cover = |grid: Grid, shift: f64| -> f64 {
            let mut c = 0f64;
            for b in 0..grid.bars {
                for step in 0..STEPS {
                    let p = (grid.pos(b, step) as f64 + shift * grid.step_frames()) / hop;
                    let (lo, hi) = ((p - 4.0).max(0.0) as usize, ((p + 4.0) as usize).min(flux.len().saturating_sub(1)));
                    if lo <= hi {
                        c += flux[lo..=hi].iter().fold(0f32, |m, x| m.max(*x)) as f64;
                    }
                }
            }
            c
        };
        let mut scored: Vec<(f64, Grid)> = Vec::new();
        for (bpm, bars) in cands {
            let grid = Grid { bpm, bars };
            let contrast = cover(grid, 0.0) - cover(grid, 0.5);
            self.say(format!("grid candidate {bpm} bpm x {bars} bars: onset lattice contrast {contrast:.3}"));
            scored.push((contrast, grid));
        }
        let top = scored.iter().map(|x| x.0).fold(f64::MIN, f64::max);
        let pick = scored.iter().filter(|x| x.0 >= 0.9 * top).min_by_key(|x| x.1.bars).unwrap();
        Ok(pick.1)
    }

    /// The best few events not in `st`, by what they would explain of what the current render leaves
    /// unexplained: the pursuit's proposals again, with every event already present subtracted.
    /// Drums also come as whole sets (every step above a fraction of the best), since a lone retriggered hit
    /// looks wrong until its neighbours are there.
    fn proposals(&self, grid: Grid, st: &State, bank: &[Template]) -> Result<Vec<(String, State)>, String> {
        let pcm = self.eval(st)?;
        let cand = spec_of(&pcm.samples, 44100.0);
        let bands = self.target.bands;
        let mut r = self.target.data.clone();
        for f in 0..cand.frames.min(self.target.frames) {
            for b in 0..bands {
                r[f * bands + b] -= cand.data[f * bands + b];
            }
        }
        let mut scored: Vec<(usize, f64, String, State)> = Vec::new();
        let mut per_role: std::collections::BTreeMap<String, Vec<(f64, usize)>> = Default::default();
        for t in bank {
            for step in 0..STEPS {
                let (class, cand) = match &t.key {
                    Key::Drum(d) => {
                        if st.drums.get(d).map_or(false, |s| s[step]) {
                            continue;
                        }
                        (0, st.with_drum(d, step))
                    }
                    Key::Bass { inst, midi } => {
                        if let Some((i, steps)) = &st.bass {
                            if i != inst || steps[step].is_some() {
                                continue;
                            }
                        }
                        (1, st.with_bass(inst, step, *midi))
                    }
                    Key::Chord { chord, inst } => {
                        if st.chord.is_some() || step != 0 {
                            continue;
                        }
                        (2, st.with_chord(chord, inst))
                    }
                };
                let p = self.predict(grid, &r, t, step);
                if p > 0.0 {
                    if let Key::Drum(d) = &t.key {
                        per_role.entry(d.clone()).or_default().push((p, step));
                    }
                    scored.push((class, p, format!("+ {:?} @ {step}", t.key), cand));
                }
            }
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let mut out = Vec::new();
        for (class, take) in [(0, 4), (1, 6), (2, 4)] {
            out.extend(scored.iter().filter(|x| x.0 == class).take(take).map(|x| (x.2.clone(), x.3.clone())));
        }
        for (role, mut v) in per_role {
            v.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
            let top = v[0].0;
            for frac in [0.5, 0.25, 0.1] {
                let mut c = st.clone();
                let mut n = 0;
                for (p, step) in &v {
                    if *p >= frac * top {
                        c = c.with_drum(&role, *step);
                        n += 1;
                    }
                }
                if n > 1 {
                    out.push((format!("+ {role} x{n} (>= {frac} of the best)"), c));
                }
            }
        }
        Ok(out)
    }

    /// Hill climb from the pursuit's program. Moves: remove an event, move a bass note, add a proposal.
    /// Each is scored by a real render of the first two bars; the best improvement is taken, and the climb
    /// stops when nothing improves by half a percent. (Two bars assume the pattern repeats per bar, which is
    /// what this model of a program is; the final score is on every bar.)
    fn hill_climb(&mut self, grid: Grid, st: &mut State, bank: &[Template]) -> Result<f64, String> {
        const PROBE: u32 = 2;
        let mut err = self.eval_probe(st, PROBE)?;
        for round in 0..30 {
            let mut moves: Vec<(String, State)> = Vec::new();
            for (role, steps) in &st.drums {
                for step in 0..STEPS {
                    if steps[step] {
                        let mut c = st.clone();
                        c.drums.get_mut(role).unwrap()[step] = false;
                        moves.push((format!("- {role} @ {step}"), c));
                    }
                }
            }
            if let Some((_, steps)) = &st.bass {
                for step in 0..STEPS {
                    if let Some(m) = steps[step] {
                        let mut c = st.clone();
                        c.bass.as_mut().unwrap().1[step] = None;
                        moves.push((format!("- bass @ {step}"), c));
                        for d in [12i32, -12, 1, -1, 2, -2] {
                            let n = m as i32 + d;
                            if (0..128).contains(&n) {
                                let mut c = st.clone();
                                c.bass.as_mut().unwrap().1[step] = Some(n as u8);
                                moves.push((format!("bass @ {step}: {} -> {}", note_name(m), note_name(n as u8)), c));
                            }
                        }
                    }
                }
            }
            moves.extend(self.proposals(grid, st, bank)?);
            moves.retain(|(_, c)| c.text() != st.text());
            if moves.is_empty() {
                break;
            }
            let scored = par_map(moves, self.opts.jobs, |(label, c)| {
                let r = self.eval_probe(&c, PROBE);
                (label, c, r)
            });
            let mut best: Option<(f64, String, State)> = None;
            for (label, c, r) in scored {
                let e = r?;
                if best.as_ref().map_or(true, |b| e < b.0) {
                    best = Some((e, label, c));
                }
            }
            let (e, label, c) = best.unwrap();
            if e < err * 0.995 {
                self.say(format!("climb {round}: {label}: spectral error {err:.3e} -> {e:.3e}"));
                *st = c;
                err = e;
            } else {
                break;
            }
        }
        Ok(err)
    }

    /// The fitted gain of every hit of `part` (in step order), all hits of the program refitted together
    /// against the target in feature space. A hit's gain tracks its velocity: it is the shape of the
    /// velocities, and the scale is unknown (the deck's own gain staging), which a render settles.
    fn hit_gains(&self, grid: Grid, st: &State, bank: &[Template], part: &str) -> Option<Vec<f64>> {
        let mut cols: Vec<(String, usize, Vec<f32>)> = Vec::new();
        for (role, steps) in &st.drums {
            let t = bank.iter().find(|t| t.key == Key::Drum(role.clone()))?;
            for step in 0..STEPS {
                if steps[step] {
                    cols.push((role.clone(), step, self.column(grid, t, step)));
                }
            }
        }
        if let Some((inst, steps)) = &st.bass {
            for step in 0..STEPS {
                if let Some(m) = steps[step] {
                    let t = bank.iter().find(|t| t.key == Key::Bass { inst: inst.clone(), midi: m })?;
                    cols.push(("bass".to_string(), step, self.column(grid, t, step)));
                }
            }
        }
        let n = cols.len();
        let gram: Vec<Vec<f64>> = (0..n).map(|i| (0..n).map(|j| self.gdot(&cols[i].2, &cols[j].2)).collect()).collect();
        let rhs: Vec<f64> = cols.iter().map(|c| self.gdot(&self.target.data, &c.2)).collect();
        let g = solve(&gram, &rhs);
        let mut out: Vec<(usize, f64)> = cols.iter().zip(&g).filter(|(c, _)| c.0 == part).map(|(c, g)| (c.1, *g)).collect();
        out.sort_by_key(|x| x.0);
        if out.is_empty() {
            None
        } else {
            Some(out.into_iter().map(|x| x.1).collect())
        }
    }

    /// Total squared sample error of the first two bars of `st`.
    fn total_probe(&self, grid: Grid, st: &State) -> Result<f64, String> {
        let n = ((grid.bar_frames() * 2.0) as usize * 2).min(self.target_pcm.samples.len());
        Ok(self.wave_error(&self.eval_n(st, 2)?, 0, n))
    }

    /// Choose a part's velocity. Candidates: the default, a fixed value, and every `vel=lo..hi` whose compiled net
    /// has the shape the fitted gains show (a range gives each hit a velocity from a hash of its place, so the
    /// range is read off the nets, not guessed). The winner is the candidate with the least real sample error over
    /// the whole program; the default stays unless something beats it by a percent.
    fn fit_velocity(&mut self, grid: Grid, st: &mut State, part: &str, bank: &[Template]) -> Result<(), String> {
        let Some(gains) = self.hit_gains(grid, st, bank, part) else { return Ok(()) };
        if gains.len() < 2 {
            return Ok(());
        }
        let set = |s: &mut State, v: Vel| {
            if part == "bass" {
                s.bass_vel = v;
            } else {
                s.vel.insert(part.to_string(), v);
            }
        };
        let (deck, role) = if part == "bass" { ("B", "bass") } else { ("A", part) };
        let hits: Vec<(usize, u8)> = if part == "bass" {
            st.bass.as_ref().map(|(_, b)| (0..STEPS).filter_map(|s| b[s].map(|m| (s, m))).collect()).unwrap_or_default()
        } else {
            st.drums.get(part).map(|d| (0..STEPS).filter(|s| d[*s]).map(|s| (s, vel::drum_note(part))).collect()).unwrap_or_default()
        };
        let g2 = gains.iter().map(|x| x * x).sum::<f64>();
        self.say(format!("velocity: {part} fitted gains {:?}", gains.iter().map(|g| (g * 1000.0).round() / 1000.0).collect::<Vec<_>>()));

        // Shape: the least-squares scale of the range's velocities onto the gains, and what is left over.
        let mut shaped: Vec<(f64, u8, u8)> = Vec::new();
        for lo in 1..=126u8 {
            for hi in (lo + 1)..=127u8 {
                let v: Vec<f64> = hits.iter().map(|(step, note)| vel::pick(deck, role, 0, *step, *note, lo, hi) as f64).collect();
                let vv: f64 = v.iter().map(|x| x * x).sum();
                let sc = v.iter().zip(&gains).map(|(a, b)| a * b).sum::<f64>() / vv;
                let resid: f64 = v.iter().zip(&gains).map(|(a, b)| (b - sc * a).powi(2)).sum::<f64>() / g2.max(1e-30);
                shaped.push((resid, lo, hi));
            }
        }
        shaped.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut cands: Vec<(String, Vel)> = vec![("default".into(), Vel::Default)];
        cands.extend((1..=16).map(|k| (format!("fixed {}", k * 8), Vel::Fixed((k * 8).min(127) as u8))));
        cands.extend(shaped.iter().take(48).map(|(r, lo, hi)| (format!("{lo}..{hi} (shape residual {r:.3})"), Vel::Range(*lo, *hi))));
        let base = st.clone();
        let scored = par_map(cands, self.opts.jobs, |(label, v)| {
            let mut c = base.clone();
            set(&mut c, v);
            (label, v, self.total_probe(grid, &c))
        });
        let mut best: Option<(f64, String, Vel)> = None;
        let mut default_err = f64::MAX;
        for (label, v, e) in scored {
            let e = e?;
            if label == "default" {
                default_err = e;
            }
            if best.as_ref().map_or(true, |b| e < b.0) {
                best = Some((e, label, v));
            }
        }
        let (e, label, v) = best.unwrap();
        if e < default_err * 0.99 {
            self.say(format!("velocity: {part} -> {label}: sample error {default_err:.3e} -> {e:.3e}"));
            set(st, v);
        } else {
            self.say(format!("velocity: {part} stays at the default (best {label}: {e:.3e} against {default_err:.3e})"));
        }
        Ok(())
    }
}

fn drum_items(bpm: f64) -> Vec<(Key, State)> {
    let base = State::new(bpm, 2);
    DRUMS.iter().map(|d| (Key::Drum(d.to_string()), base.with_drum(d, 0))).collect()
}

/// The pursuit's selection as a program: drums as found; the bass voice that explains the most, one note a step.
fn state_from_selection(grid: Grid, bank: &[Template], sel: &[(usize, usize, f64)], min_gain: f64) -> State {
    let mut st = State::new(grid.bpm, grid.bars);
    let mut by_inst: Vec<(String, f64)> = Vec::new();
    for (ti, _, g) in sel {
        let t = &bank[*ti];
        if let Key::Bass { inst, .. } = &t.key {
            let power = g * g * t.norm2;
            match by_inst.iter_mut().find(|x| &x.0 == inst) {
                Some(x) => x.1 += power,
                None => by_inst.push((inst.clone(), power)),
            }
        }
    }
    let bass_inst = by_inst.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).map(|x| x.0.clone());
    let mut bass_events: Vec<(f64, usize, u8, String)> = Vec::new();
    for (ti, step, g) in sel {
        if *g < min_gain {
            continue;
        }
        match &bank[*ti].key {
            Key::Drum(d) => st = st.with_drum(d, *step),
            Key::Bass { inst, midi } if Some(inst) == bass_inst.as_ref() => bass_events.push((*g, *step, *midi, inst.clone())),
            _ => {}
        }
    }
    bass_events.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let mut taken = [false; STEPS];
    for (_, step, midi, inst) in bass_events {
        if !taken[step] {
            taken[step] = true;
            st = st.with_bass(&inst, step, midi);
        }
    }
    st
}

/// Recover a program from a render. `reference` is the original program text, for L1/L2 scoring only:
/// it is never read by the recovery.
pub fn recover(audio: &Pcm, btwd: &Btwd, opts: &Options, reference: Option<&str>) -> Result<Report, String> {
    if audio.sr != 44100 {
        return Err(format!("{} Hz: only 44100 (resample first)", audio.sr));
    }
    let target = spec_of(&audio.samples, 44100.0);
    if target.frames == 0 {
        return Err("too short".into());
    }
    let w = band_weights(&target);
    let target_energy: f64 = target.data.iter().enumerate().map(|(i, x)| (x * x * w[i % target.bands]) as f64).sum();
    if target_energy == 0.0 {
        return Err("silence".into());
    }
    let mut rc = Recoverer { btwd, opts, target_pcm: audio, target, w, target_energy, log: Vec::new() };

    let grid = rc.pick_grid(audio.frames())?;
    rc.say(format!("grid: {} bpm, {} bars", grid.bpm, grid.bars));

    // One bank: drums, every bass voice at every pitch, every triad on every pad voice.
    let base = State::new(grid.bpm, 2);
    let mut bank = rc.bank(drum_items(grid.bpm), 4.0);
    let mut bass_items = Vec::new();
    for inst in &opts.bass_voices {
        for midi in opts.bass_notes.0..=opts.bass_notes.1 {
            bass_items.push((Key::Bass { inst: inst.clone(), midi }, base.with_bass(inst, 0, midi)));
        }
    }
    bank.extend(rc.bank(bass_items, 4.0));
    let mut pad_items = Vec::new();
    for inst in &opts.pad_voices {
        for chord in triads() {
            pad_items.push((Key::Chord { chord: chord.clone(), inst: inst.clone() }, base.with_chord(&chord, inst)));
        }
    }
    bank.extend(rc.bank(pad_items, 16.0));
    rc.say(format!("bank: {} templates", bank.len()));

    // The pad is far quieter than the rest: the pursuit leaves it to the climb, which sees what is left.
    let (sel, resid) = rc.omp(grid, &bank, &|k: &Key, _| !matches!(k, Key::Chord { .. }), 24, 2e-3);
    rc.say(format!("pursuit: {} columns, model residual {:.2e} of the signal", sel.len(), resid / rc.target_energy));
    for (ti, step, g) in &sel {
        rc.say(format!("  {:?} @ {step}: gain {g:.3}", bank[*ti].key));
    }
    let mut st = state_from_selection(grid, &bank, &sel, 0.15);
    rc.say(format!("pursuit program:\n{}", st.text()));

    let err = rc.hill_climb(grid, &mut st, &bank)?;
    rc.say(format!("after climb: spectral error {err:.3e}"));

    // Velocities, in the order the parts are heard: kick, snare, hihat, clap, then the bass.
    for part in ["hihat", "snare", "kick", "clap", "bass"] {
        rc.fit_velocity(grid, &mut st, part, &bank)?;
    }
    // With the velocities right the quiet parts show: climb again (this is where a pad is found), then read the
    // velocities once more on the fuller program.
    let before = st.text();
    let err = rc.hill_climb(grid, &mut st, &bank)?;
    rc.say(format!("after velocities and a second climb: spectral error {err:.3e}"));
    if st.text() != before {
        for part in ["hihat", "snare", "kick", "clap", "bass"] {
            rc.fit_velocity(grid, &mut st, part, &bank)?;
        }
    }

    let text = st.text();
    let final_pcm = rc.eval(&st)?;
    let l0 = score::l0(audio, &final_pcm);
    rc.say(format!("final: L0 max diff {}, {} samples differ", l0.max_diff, l0.differing));
    let same = match reference {
        Some(r) => Some(btwd::same_as(&text, r)?),
        None => None,
    };
    Ok(Report { text, l0, same_as: same, log: rc.log })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_from_length() {
        // first-beat: 8 bars at 124 bpm
        let frames = (8.0f64 * 240.0 / 124.0 * 44100.0).round() as usize;
        let c = grid_candidates(frames, 44100);
        assert!(c.contains(&(124.0, 8)), "{c:?}");
        assert!(c.contains(&(62.0, 4)), "the family shares a length: {c:?}");
        assert!(grid_candidates(12345, 44100).is_empty());
    }

    #[test]
    fn grid_positions() {
        let g = Grid { bpm: 124.0, bars: 8 };
        assert_eq!(g.pos(0, 0), 0);
        // the renderer starts an event on the next whole frame: 5334.67 -> 5335, 16004.03 -> 16005
        assert_eq!(g.pos(0, 1), 5335);
        assert_eq!(g.pos(0, 3), 16005);
    }

    #[test]
    fn solve_small() {
        let g = vec![vec![2.0, 1.0], vec![1.0, 3.0]];
        let x = solve(&g, &[3.0, 5.0]);
        assert!((x[0] - 0.8).abs() < 1e-6 && (x[1] - 1.4).abs() < 1e-6, "{x:?}");
    }
}
