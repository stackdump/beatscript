//! Set recording on the host (docs/RECORDING.md): the master output to a WAV file on a worker
//! thread, the recorded sets kept by CID, and the booth's signature over them.
//!
//! - [`Recorder`] tails the engine's master tap (`BusTaps::master`, the output after the master
//!   fader, a ~22 s ring) every 100 ms into a 32-bit float WAV, so the audio thread never waits
//!   on the disk. A reader that falls behind the ring gets silence for the frames it lost (the
//!   tap's own rule) and the recorder counts them as `dropped`. RIFF sizes are 32-bit: the
//!   recorder stops itself at 4 GB (about 3 h 06 min at 48 kHz stereo) and says so.
//! - [`SetStore`] keeps each recorded set as `<support>/recordings/<cid>/`: `set.json` (the
//!   canonical object, `djengine::setrec::RecordedSet`), `set.wav` (the take: its content ID is
//!   in `set.json`) and `signature.json`.
//! - [`Identity`] is the booth's signing key: secp256k1 (djrust-wallet, the bitwrap identity
//!   wallet core), kept at `<support>/identity/booth.key` (0600; `$DJ_IDENTITY_KEY` names
//!   another file), made on first use. A set is signed EIP-191 `personal_sign` over its CID's
//!   text, and the signer is a `did:pkh:eip155:1:0x…`: anyone can recover the address from the
//!   signature and the CID and compare.

use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use djengine::json::{JsonObject, JsonValue};
use djengine::setrec::{raw_cid_of_digest, AudioRef, RecordedSet};
use sha2::{Digest, Sha256};

use crate::deck::capture::BusTaps;
use crate::stores::{env_or_support, pretty_sorted, read_json, write_atomic};

// ---------------------------------------------------------------------------------------------
// WAV

/// RIFF's size fields are 32-bit.
pub const WAV_LIMIT: u64 = u32::MAX as u64 - 64;

/// A float32 stereo WAV written as it goes; the sizes are filled in by [`WavWriter::finish`].
pub struct WavWriter {
    out: BufWriter<File>,
    pub path: PathBuf,
    pub frames: u64,
    pub sample_rate: u32,
}

impl WavWriter {
    pub fn create(path: &Path, sample_rate: u32) -> std::io::Result<WavWriter> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut out = BufWriter::with_capacity(1 << 20, File::create(path)?);
        out.write_all(&header(0, sample_rate))?;
        Ok(WavWriter { out, path: path.to_path_buf(), frames: 0, sample_rate })
    }

    /// Bytes of audio written.
    pub fn data_bytes(&self) -> u64 {
        self.frames * 8
    }

    /// Interleave and append. Refuses (false) past [`WAV_LIMIT`].
    pub fn write(&mut self, left: &[f32], right: &[f32]) -> std::io::Result<bool> {
        let n = left.len().min(right.len());
        if self.data_bytes() + n as u64 * 8 > WAV_LIMIT {
            return Ok(false);
        }
        let mut buf = Vec::with_capacity(n * 8);
        for i in 0..n {
            buf.extend_from_slice(&left[i].to_le_bytes());
            buf.extend_from_slice(&right[i].to_le_bytes());
        }
        self.out.write_all(&buf)?;
        self.frames += n as u64;
        Ok(true)
    }

    /// Patch the sizes, close, and answer the file's sha-256.
    pub fn finish(mut self) -> std::io::Result<(PathBuf, u64, [u8; 32])> {
        self.out.flush()?;
        let mut f = self.out.into_inner().map_err(|e| e.into_error())?;
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&header(self.frames * 8, self.sample_rate))?;
        f.sync_all()?;
        drop(f);
        Ok((self.path.clone(), self.frames, sha256_file(&self.path)?))
    }
}

/// A 44-byte WAVE_FORMAT_IEEE_FLOAT (3) stereo header for `data` bytes.
fn header(data: u64, sr: u32) -> [u8; 44] {
    let data = data.min(u32::MAX as u64 - 36) as u32;
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&3u16.to_le_bytes());
    h[22..24].copy_from_slice(&2u16.to_le_bytes());
    h[24..28].copy_from_slice(&sr.to_le_bytes());
    h[28..32].copy_from_slice(&(sr * 8).to_le_bytes());
    h[32..34].copy_from_slice(&8u16.to_le_bytes());
    h[34..36].copy_from_slice(&32u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

/// The sha-256 of a file, read in 1 MB pieces.
pub fn sha256_file(path: &Path) -> std::io::Result<[u8; 32]> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
}

/// A file's content ID (CIDv1 raw, sha2-256).
pub fn file_cid(path: &Path) -> std::io::Result<String> {
    Ok(raw_cid_of_digest(&sha256_file(path)?))
}

// ---------------------------------------------------------------------------------------------
// The recorder

/// What a recorder thread shares with the window.
#[derive(Default)]
pub struct RecorderShared {
    pub stop: AtomicBool,
    pub frames: AtomicU64,
    pub dropped: AtomicU64,
    /// It stopped itself (the file limit) or failed: why.
    pub ended: Mutex<Option<String>>,
}

/// The finished take.
#[derive(Clone, Debug, PartialEq)]
pub struct Take {
    pub path: PathBuf,
    pub frames: u64,
    pub dropped: u64,
    pub sample_rate: f64,
    pub audio: AudioRef,
}

/// The master output, recording.
pub struct Recorder {
    pub shared: Arc<RecorderShared>,
    pub path: PathBuf,
    pub sample_rate: f64,
    /// The tap frame it started at.
    pub start_frame: i64,
    thread: Option<JoinHandle<std::io::Result<(PathBuf, u64, [u8; 32])>>>,
}

impl Recorder {
    /// Start writing the master tap to `path` from now.
    pub fn start(taps: Arc<BusTaps>, path: &Path, sample_rate: f64) -> std::io::Result<Recorder> {
        let mut wav = WavWriter::create(path, sample_rate.round() as u32)?;
        let shared = Arc::new(RecorderShared::default());
        let start_frame = taps.master.span().map_or(0, |(_, end)| end);
        let sh = shared.clone();
        let thread = std::thread::Builder::new().name("set-recorder".into()).spawn(move || {
            let mut cursor = start_frame;
            let (mut l, mut r) = (Vec::new(), Vec::new());
            loop {
                let stopping = sh.stop.load(Ordering::Acquire);
                if let Some((oldest, end)) = taps.master.span() {
                    if cursor < oldest {
                        sh.dropped.fetch_add((oldest - cursor) as u64, Ordering::Relaxed);
                    }
                    if end > cursor {
                        l.clear();
                        r.clear();
                        cursor = taps.master.copy(cursor, end, &mut l, &mut r);
                        if !wav.write(&l, &r)? {
                            *sh.ended.lock().unwrap() =
                                Some("the WAV reached 4 GB (RIFF's limit): the take stopped there".into());
                            break;
                        }
                        sh.frames.store(wav.frames, Ordering::Relaxed);
                    }
                }
                if stopping {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            wav.finish()
        })?;
        Ok(Recorder { shared, path: path.to_path_buf(), sample_rate, start_frame, thread: Some(thread) })
    }

    pub fn frames(&self) -> u64 {
        self.shared.frames.load(Ordering::Relaxed)
    }

    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / self.sample_rate.max(1.0)
    }

    /// Stopped by itself (the size limit): why.
    pub fn ended(&self) -> Option<String> {
        self.shared.ended.lock().ok().and_then(|g| g.clone())
    }

    /// Stop, flush, close and hash the file.
    pub fn stop(mut self) -> Result<Take, String> {
        self.shared.stop.store(true, Ordering::Release);
        let (path, frames, digest) = self
            .thread
            .take()
            .ok_or("record: not recording")?
            .join()
            .map_err(|_| "record: the recorder thread panicked".to_string())?
            .map_err(|e| format!("record: {e}"))?;
        let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Take {
            path,
            frames,
            dropped: self.shared.dropped.load(Ordering::Relaxed),
            sample_rate: self.sample_rate,
            audio: AudioRef {
                cid: raw_cid_of_digest(&digest),
                file,
                frames,
                sample_rate: self.sample_rate,
                channels: 2,
                format: "wav-f32".into(),
            },
        })
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Identity

/// The booth's signing key.
pub struct Identity {
    secret: zeroize::Zeroizing<[u8; 32]>,
    pub address: String,
    pub did: String,
}

impl Identity {
    pub fn default_path() -> PathBuf {
        match std::env::var_os("DJ_IDENTITY_KEY") {
            Some(p) if !p.is_empty() => PathBuf::from(p),
            _ => crate::stores::support_dir().join("identity").join("booth.key"),
        }
    }

    /// The key at `path`, made (and written 0600) when there is none.
    pub fn load_or_create(path: &Path) -> Result<Identity, String> {
        let hexkey = match std::fs::read_to_string(path) {
            Ok(t) => t.trim().to_string(),
            Err(_) => {
                let k = djrust_wallet::key::generate_private_key().map_err(|e| format!("identity: {e:?}"))?;
                let text = djrust_wallet::hex::encode(&k[..]);
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d).map_err(|e| format!("identity: {e}"))?;
                }
                write_secret(path, text.as_bytes()).map_err(|e| format!("identity: {e}"))?;
                text
            }
        };
        let bytes = djrust_wallet::hex::decode(hexkey.trim_start_matches("0x")).map_err(|e| format!("identity: {e:?}"))?;
        if bytes.len() != 32 || !djrust_wallet::key::is_valid_private_key(&bytes) {
            return Err(format!("identity: {} is not a secp256k1 key", path.display()));
        }
        let mut secret = zeroize::Zeroizing::new([0u8; 32]);
        secret.copy_from_slice(&bytes);
        let address = djrust_wallet::key::address(&secret[..]).map_err(|e| format!("identity: {e:?}"))?;
        let did = djrust_wallet::key::did::pkh(&address);
        Ok(Identity { secret, address, did })
    }

    pub fn open_default() -> Result<Identity, String> {
        Identity::load_or_create(&Identity::default_path())
    }

    /// EIP-191 personal_sign over `message`: `0x…` 65 bytes.
    pub fn sign(&self, message: &str) -> Result<String, String> {
        let h = djrust_wallet::key::personal_message_hash(message.as_bytes());
        let s = djrust_wallet::key::sign(&h, &self.secret[..]).map_err(|e| format!("sign: {e:?}"))?;
        Ok(s.hex())
    }

    /// `{cid, did, alg, sig}`: a signature over a CID.
    pub fn sign_cid(&self, cid: &str) -> Result<JsonValue, String> {
        Ok(JsonValue::Object(JsonObject::from_pairs([
            ("cid", JsonValue::str(cid)),
            ("did", JsonValue::String(self.did.clone())),
            ("alg", JsonValue::str(SIGNATURE_ALG)),
            ("sig", JsonValue::String(self.sign(cid)?)),
        ])))
    }
}

pub const SIGNATURE_ALG: &str = "eip191-secp256k1";

fn write_secret(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
        f.write_all(bytes)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

/// Does `sig` (`{cid, did, alg, sig}`) sign `cid`, by the key its `did` names?
pub fn verify_signature(cid: &str, sig: &JsonValue) -> Result<String, String> {
    let s = |k: &str| sig.get(k).and_then(JsonValue::as_str).ok_or_else(|| format!("signature: no {k}"));
    if s("cid")? != cid {
        return Err(format!("signature: it signs {}, not {cid}", s("cid")?));
    }
    if s("alg")? != SIGNATURE_ALG {
        return Err(format!("signature: alg {} (want {SIGNATURE_ALG})", s("alg")?));
    }
    let did = s("did")?;
    let (_, want) = djrust_wallet::key::did::parse(did).ok_or("signature: did is not a did:pkh:eip155")?;
    let es = djrust_wallet::key::EthSignature::from_hex(s("sig")?).map_err(|e| format!("signature: {e:?}"))?;
    let got = djrust_wallet::key::recover_address(&djrust_wallet::key::personal_message_hash(cid.as_bytes()), &es)
        .ok_or("signature: does not recover")?;
    if got != want {
        return Err("signature: signed by another key than its did".into());
    }
    Ok(did.to_string())
}

// ---------------------------------------------------------------------------------------------
// The store

pub struct SetStore {
    pub directory: PathBuf,
}

impl SetStore {
    pub fn new(directory: impl Into<PathBuf>) -> SetStore {
        SetStore { directory: directory.into() }
    }

    pub fn default_directory() -> PathBuf {
        env_or_support("DJ_RECORDINGS_DIR", "recordings")
    }

    pub fn open_default() -> SetStore {
        SetStore::new(SetStore::default_directory())
    }

    pub fn dir(&self, cid: &str) -> PathBuf {
        self.directory.join(cid)
    }

    /// Where a take is written while it records (moved into the set's folder at save).
    pub fn take_path(&self, stamp: &str) -> PathBuf {
        self.directory.join(".takes").join(format!("{stamp}.wav"))
    }

    /// Save `set` (its audio file moved in from `take` when given) and its signature. Answers the
    /// CID.
    pub fn save(&self, set: &RecordedSet, take: Option<&Path>, signature: Option<&JsonValue>) -> Result<String, String> {
        let cid = set.cid();
        let dir = self.dir(&cid);
        std::fs::create_dir_all(&dir).map_err(|e| format!("record: {e}"))?;
        if let (Some(t), Some(a)) = (take, set.audio.as_ref()) {
            let dest = dir.join(&a.file);
            if t != dest {
                std::fs::rename(t, &dest)
                    .or_else(|_| std::fs::copy(t, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(t)))
                    .map_err(|e| format!("record: moving the take: {e}"))?;
            }
        }
        write_atomic(&dir.join("set.json"), pretty_sorted(&set.to_json()).as_bytes()).map_err(|e| format!("record: {e}"))?;
        if let Some(s) = signature {
            write_atomic(&dir.join("signature.json"), pretty_sorted(s).as_bytes()).map_err(|e| format!("record: {e}"))?;
        }
        Ok(cid)
    }

    pub fn get(&self, cid: &str) -> Option<RecordedSet> {
        RecordedSet::from_json(&read_json(&self.dir(cid).join("set.json"))?).ok()
    }

    pub fn signature(&self, cid: &str) -> Option<JsonValue> {
        read_json(&self.dir(cid).join("signature.json"))
    }

    pub fn audio_path(&self, cid: &str) -> Option<PathBuf> {
        let s = self.get(cid)?;
        let p = self.dir(cid).join(&s.audio?.file);
        p.is_file().then_some(p)
    }

    /// Every recorded set whose folder is named by its CID, newest first.
    pub fn list(&self) -> Vec<RecordedSet> {
        let Ok(rd) = std::fs::read_dir(&self.directory) else { return Vec::new() };
        let mut v: Vec<RecordedSet> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .filter_map(|cid| self.get(&cid).filter(|s| s.cid() == cid))
            .collect();
        v.sort_by(|a, b| b.started.cmp(&a.started).then(a.name.cmp(&b.name)));
        v
    }

    /// A whole CID, its head or tail, or a name.
    pub fn resolve(&self, query: &str) -> Result<RecordedSet, String> {
        let q = query.trim();
        let all = self.list();
        if q.is_empty() {
            return all.into_iter().next().ok_or_else(|| "record: no recorded sets".into());
        }
        let lq = q.to_lowercase();
        let tiers: [&dyn Fn(&RecordedSet, &str) -> bool; 3] = [
            &|_, c| c == q,
            &|_, c| c.starts_with(q) || c.ends_with(q),
            &|s, _| s.name.to_lowercase().contains(&lq),
        ];
        for t in tiers {
            let hits: Vec<&RecordedSet> = all.iter().filter(|s| t(s, &s.cid())).collect();
            match hits.len() {
                0 => continue,
                1 => return Ok(hits[0].clone()),
                n => return Err(format!("record: {q:?} matches {n} sets; give more of the cid")),
            }
        }
        Err(format!("record: no recorded set {q:?}"))
    }

    /// Check a saved set end to end: the folder is its CID, the audio file hashes to the CID the
    /// set names, and the signature (if any) recovers to its DID. Answers what holds.
    pub fn verify(&self, cid: &str) -> Result<JsonValue, String> {
        let set = self.get(cid).ok_or_else(|| format!("record: no recorded set {cid}"))?;
        let mut o = JsonObject::new();
        o.set("cid", JsonValue::str(cid));
        o.set("cid_ok", JsonValue::Bool(set.cid() == cid));
        match &set.audio {
            Some(a) => {
                let p = self.dir(cid).join(&a.file);
                let got = file_cid(&p).map_err(|e| format!("record: {}: {e}", p.display()))?;
                o.set("audio_ok", JsonValue::Bool(got == a.cid));
            }
            None => o.set("audio_ok", JsonValue::Null),
        }
        match self.signature(cid) {
            Some(sig) => match verify_signature(cid, &sig) {
                Ok(did) => {
                    o.set("signed_by", JsonValue::String(did));
                    o.set("signature_ok", JsonValue::Bool(true));
                }
                Err(e) => {
                    o.set("signature_ok", JsonValue::Bool(false));
                    o.set("signature_error", JsonValue::String(e));
                }
            },
            None => o.set("signature_ok", JsonValue::Null),
        }
        Ok(JsonValue::Object(o))
    }
}

/// Now as ISO 8601 UTC ("2026-11-17T21:04:05Z"), from the system clock (civil-from-days).
pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    iso_of(secs)
}

pub fn iso_of(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, (rem / 60) % 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("djsetrec-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_recorder_writes_what_the_master_tap_played() {
        let taps = Arc::new(BusTaps::default());
        let dir = tmp("rec");
        let path = dir.join("take.wav");
        // the tap has played a little before Record: that is not in the take
        taps.master.write(0, &[0.5; 512], &[0.5; 512]);
        let rec = Recorder::start(taps.clone(), &path, 48_000.0).unwrap();
        let l: Vec<f32> = (0..48_000).map(|i| (i as f32 / 48_000.0) - 0.5).collect();
        let r: Vec<f32> = l.iter().map(|v| -v).collect();
        let mut f = 512i64;
        for chunk in 0..(48_000 / 4800) {
            let a = chunk * 4800;
            taps.master.write(f, &l[a..a + 4800], &r[a..a + 4800]);
            f += 4800;
        }
        let take = rec.stop().unwrap();
        assert_eq!(take.frames, 48_000);
        assert_eq!(take.dropped, 0);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as u64, 48_000 * 8);
        let first = f32::from_le_bytes(bytes[44..48].try_into().unwrap());
        let first_r = f32::from_le_bytes(bytes[48..52].try_into().unwrap());
        assert_eq!((first, first_r), (l[0], r[0]));
        assert_eq!(take.audio.cid, djengine::setrec::raw_cid(&bytes));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn signatures_recover_to_the_signer_and_nothing_else() {
        let dir = tmp("id");
        let key = dir.join("booth.key");
        let id = Identity::load_or_create(&key).unwrap();
        assert!(id.did.starts_with("did:pkh:eip155:1:0x"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&key).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let again = Identity::load_or_create(&key).unwrap();
        assert_eq!(again.did, id.did, "the key is kept");
        let sig = id.sign_cid("zSomeCid").unwrap();
        assert_eq!(verify_signature("zSomeCid", &sig).unwrap(), id.did);
        assert!(verify_signature("zOther", &sig).is_err());
        let mut forged = match sig.clone() {
            JsonValue::Object(o) => o,
            _ => unreachable!(),
        };
        let other = Identity::load_or_create(&dir.join("other.key")).unwrap();
        forged.set("did", JsonValue::String(other.did.clone()));
        assert!(verify_signature("zSomeCid", &JsonValue::Object(forged)).unwrap_err().contains("another key"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_saved_set_verifies_end_to_end() {
        let dir = tmp("store");
        let store = SetStore::new(&dir);
        let take = store.take_path("t1");
        let mut w = WavWriter::create(&take, 48_000).unwrap();
        w.write(&[0.25; 4800], &[-0.25; 4800]).unwrap();
        let (p, frames, digest) = w.finish().unwrap();
        assert_eq!(p, take);
        let set = RecordedSet {
            name: "test".into(),
            started: iso_of(1_795_000_000),
            bpm: 120.0,
            duration: frames as f64 / 48_000.0,
            audio: Some(AudioRef {
                cid: raw_cid_of_digest(&digest),
                file: "set.wav".into(),
                frames,
                sample_rate: 48_000.0,
                channels: 2,
                format: "wav-f32".into(),
            }),
            tracklist: Vec::new(),
            arrangement: JsonValue::Null,
            bars: (1, 2),
            audio_offset: 0.0,
            forked_from: None,
        };
        let id = Identity::load_or_create(&dir.join("k")).unwrap();
        let cid = store.save(&set, Some(&take), Some(&id.sign_cid(&set.cid()).unwrap())).unwrap();
        assert!(!take.exists(), "the take moved into the set's folder");
        let v = store.verify(&cid).unwrap();
        for k in ["cid_ok", "audio_ok", "signature_ok"] {
            assert_eq!(v.get(k).and_then(JsonValue::as_bool), Some(true), "{k}: {}", v.canonical_js());
        }
        assert_eq!(store.resolve("tes").unwrap().cid(), cid);
        // tamper with the audio: the CID no longer matches
        std::fs::write(store.dir(&cid).join("set.wav"), b"nope").unwrap();
        assert_eq!(store.verify(&cid).unwrap().get("audio_ok").and_then(JsonValue::as_bool), Some(false));
        assert_eq!(iso_of(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_of(1_795_000_000), "2026-11-18T11:06:40Z");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
