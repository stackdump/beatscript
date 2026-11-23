//! Durable storage: one JSON file per account, `<data>/playlists/<key>.json`, written atomically.
//!
//! Why files and not SQLite: every read and write here is "one account's whole set of playlists" (capped at
//! `max_account_bytes`, 512 KiB by default), which is exactly one small file; the Go server keeps its own state the
//! same way (`tokens.json`, `devices.json`, `programs/<cid>.json`: JSON written to a temp file, then renamed), so an
//! operator backs up, inspects and restores both servers' data the same way; deleting an account's data is deleting
//! one file; and it adds no C dependency (rusqlite bundles SQLite's C, and beatscript-serve is built without a C toolchain,
//! see its README). What a database would give, concurrency across processes and queries across accounts, is not
//! needed: only beatscript-serve writes these files, writes for one account are serialised by a per-account lock, and the
//! one cross-account lookup (share token -> playlist) is an in-memory index rebuilt from the files at start.
//!
//! `<key>` is `hex(sha256("btw-playlists-v1\n" ‖ uid))[:32]`: a uid has a ':' in it, and the file name should not
//! say whose account it is. The owner is inside the file (0600).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use super::model::{AccountFile, Limits, Playlist};
use super::ApiError;

pub struct Store {
    dir: PathBuf,
    limits: Limits,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// bytes on disk per account key
    sizes: HashMap<String, u64>,
    total: u64,
    /// share token -> (owner uid, playlist id)
    shares: HashMap<String, (String, String)>,
}

pub fn account_key(uid: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"btw-playlists-v1\n");
    h.update(uid.as_bytes());
    hex::encode(&h.finalize()[..16])
}

/// Write `bytes` to `path` atomically: a temp file beside it (0600), fsync, rename, fsync the directory.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap();
    let tmp = dir.join(format!(".{}.tmp-{}", path.file_name().unwrap().to_string_lossy(), super::model::rand_b64(6)));
    let res = (|| {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

impl Store {
    /// Open (and make) `<data>/playlists`, reading every account file once for the share index and the sizes.
    /// A file that does not parse is left alone and reported, never overwritten.
    pub fn open(data: &Path, limits: Limits) -> std::io::Result<(Store, Vec<String>)> {
        let dir = data.join("playlists");
        std::fs::create_dir_all(&dir)?;
        let mut inner = Inner::default();
        let mut warnings = vec![];
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().to_string();
            let Some(key) = name.strip_suffix(".json") else { continue };
            if name.starts_with('.') {
                let _ = std::fs::remove_file(e.path()); // a temp file from a write that did not finish
                continue;
            }
            let b = std::fs::read(e.path())?;
            match serde_json::from_slice::<AccountFile>(&b) {
                Ok(f) if account_key(&f.owner) == key => {
                    inner.sizes.insert(key.to_string(), b.len() as u64);
                    inner.total += b.len() as u64;
                    for p in &f.playlists {
                        if let Some(t) = &p.share_token {
                            inner.shares.insert(t.clone(), (f.owner.clone(), p.id.clone()));
                        }
                    }
                }
                Ok(_) => warnings.push(format!("{}: its owner does not hash to its name; ignored", e.path().display())),
                Err(err) => warnings.push(format!("{}: {err}; ignored (left as it is)", e.path().display())),
            }
        }
        Ok((Store { dir, limits, locks: Mutex::new(HashMap::new()), inner: Mutex::new(inner) }, warnings))
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    fn lock(&self, key: &str) -> Arc<Mutex<()>> {
        self.locks.lock().unwrap().entry(key.to_string()).or_default().clone()
    }

    fn load(&self, uid: &str, key: &str) -> Result<AccountFile, ApiError> {
        match std::fs::read(self.path(key)) {
            Ok(b) => serde_json::from_slice(&b).map_err(|e| {
                eprintln!("beatscript-serve: playlists: account file {key} does not parse: {e}");
                ApiError::new(500, "internal", "this account's playlists could not be read")
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AccountFile { v: 1, owner: uid.to_string(), playlists: vec![] }),
            Err(e) => {
                eprintln!("beatscript-serve: playlists: reading account file {key}: {e}");
                Err(ApiError::new(500, "internal", "this account's playlists could not be read"))
            }
        }
    }

    /// The account's playlists (an account with none has an empty file that is never written).
    pub fn read(&self, uid: &str) -> Result<AccountFile, ApiError> {
        let key = account_key(uid);
        let l = self.lock(&key);
        let _g = l.lock().unwrap();
        self.load(uid, &key)
    }

    /// Change one account's playlists: `f` edits the file; every playlist is re-sealed (version), the caps are
    /// checked, and the file is replaced atomically. Nothing is written when `f` fails or a cap is hit.
    pub fn update<T>(&self, uid: &str, f: impl FnOnce(&mut AccountFile) -> Result<T, ApiError>) -> Result<T, ApiError> {
        let key = account_key(uid);
        let l = self.lock(&key);
        let _g = l.lock().unwrap();
        let mut file = self.load(uid, &key)?;
        let out = f(&mut file)?;
        file.v = 1;
        file.owner = uid.to_string();
        for p in &mut file.playlists {
            p.seal();
        }
        if file.playlists.len() > self.limits.max_playlists {
            return Err(ApiError::new(422, "limit", format!("at most {} playlists per account", self.limits.max_playlists)));
        }
        let bytes = serde_json::to_vec(&file).unwrap();
        if bytes.len() > self.limits.max_account_bytes {
            return Err(ApiError::new(413, "too_large", format!("this account's playlists would take {} bytes; the limit is {}", bytes.len(), self.limits.max_account_bytes)));
        }
        {
            let inner = self.inner.lock().unwrap();
            let old = inner.sizes.get(&key).copied().unwrap_or(0);
            if inner.total - old + bytes.len() as u64 > self.limits.max_total_bytes && bytes.len() as u64 > old {
                eprintln!("beatscript-serve: playlists: the storage cap ({} bytes) is reached", self.limits.max_total_bytes);
                return Err(ApiError::new(507, "storage_full", "playlist storage is full on this server; try again later").retry(3600));
            }
        }
        if file.playlists.is_empty() {
            match std::fs::remove_file(self.path(&key)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    eprintln!("beatscript-serve: playlists: removing account file {key}: {e}");
                    return Err(ApiError::new(500, "internal", "could not store the change; try again later"));
                }
            }
        } else if let Err(e) = write_atomic(&self.path(&key), &bytes) {
            eprintln!("beatscript-serve: playlists: writing account file {key}: {e}");
            return Err(ApiError::new(500, "internal", "could not store the change; try again later"));
        }
        let mut inner = self.inner.lock().unwrap();
        let old = inner.sizes.remove(&key).unwrap_or(0);
        inner.total -= old;
        if !file.playlists.is_empty() {
            inner.sizes.insert(key.clone(), bytes.len() as u64);
            inner.total += bytes.len() as u64;
        }
        inner.shares.retain(|_, (o, _)| o != uid);
        for p in &file.playlists {
            if let Some(t) = &p.share_token {
                inner.shares.insert(t.clone(), (uid.to_string(), p.id.clone()));
            }
        }
        Ok(out)
    }

    /// The playlist a share token opens, when it is still shared.
    pub fn resolve(&self, token: &str) -> Option<Playlist> {
        let (owner, id) = self.inner.lock().unwrap().shares.get(token).cloned()?;
        let f = self.read(&owner).ok()?;
        f.playlists.into_iter().find(|p| p.id == id && p.shared() && p.share_token.as_deref() == Some(token))
    }

    pub fn total_bytes(&self) -> u64 {
        self.inner.lock().unwrap().total
    }
}
