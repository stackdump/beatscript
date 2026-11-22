//! What a djnet side keeps on disk (docs/NET-PROTOCOL.md §2, §5): its own id, and the tokens of
//! the sides it has paired with. One JSON file, 0600. With no path (tests, `--config off`
//! runs) it lives in memory only.

use std::path::PathBuf;

use djengine::json::{JsonObject, JsonValue};

use crate::stores::{read_json, write_atomic};

/// A side this one has paired with.
#[derive(Clone, Debug, PartialEq)]
pub struct Paired {
    pub id: String,
    pub name: String,
    pub token: String,
    /// Unix seconds.
    pub added: f64,
}

#[derive(Clone, Debug)]
pub struct Store {
    pub path: Option<PathBuf>,
    pub id: String,
    pub paired: Vec<Paired>,
}

impl Store {
    /// The default file: `<support>/djnet.json`.
    pub fn default_path() -> PathBuf {
        crate::stores::support_dir().join("djnet.json")
    }

    /// Read `path` (a new id when there is none, written back at once).
    pub fn open(path: Option<PathBuf>) -> Store {
        let doc = path.as_deref().and_then(read_json);
        let id = doc
            .as_ref()
            .and_then(|d| d.get("id"))
            .and_then(JsonValue::as_str)
            .filter(|s| s.len() >= 16)
            .map(String::from);
        let paired = doc
            .as_ref()
            .and_then(|d| d.get("paired"))
            .and_then(JsonValue::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        let s = |k: &str| p.get(k).and_then(JsonValue::as_str).map(String::from);
                        Some(Paired {
                            id: s("id")?,
                            name: s("name").unwrap_or_default(),
                            token: s("token")?,
                            added: p.get("added").and_then(JsonValue::as_f64).unwrap_or(0.0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let fresh = id.is_none();
        let st = Store {
            path,
            id: id.unwrap_or_else(|| super::proto::random_hex(16)),
            paired,
        };
        if fresh {
            let _ = st.save();
        }
        st
    }

    pub fn token_of(&self, id: &str) -> Option<&str> {
        self.paired.iter().find(|p| p.id == id).map(|p| p.token.as_str())
    }

    /// Keep `token` for `id` (replacing one it had).
    pub fn put(&mut self, id: &str, name: &str, token: &str) {
        self.paired.retain(|p| p.id != id);
        self.paired.push(Paired {
            id: id.into(),
            name: name.into(),
            token: token.into(),
            added: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs() as f64),
        });
        let _ = self.save();
    }

    /// Forget `id` (`all`: every side). How many went.
    pub fn forget(&mut self, id: &str) -> usize {
        let before = self.paired.len();
        if id == "all" {
            self.paired.clear();
        } else {
            self.paired.retain(|p| p.id != id && p.name != id);
        }
        let _ = self.save();
        before - self.paired.len()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let paired = self
            .paired
            .iter()
            .map(|p| {
                JsonValue::Object(JsonObject::from_pairs([
                    ("id", JsonValue::str(p.id.clone())),
                    ("name", JsonValue::str(p.name.clone())),
                    ("token", JsonValue::str(p.token.clone())),
                    ("added", JsonValue::from(p.added)),
                ]))
            })
            .collect();
        let doc = JsonValue::Object(JsonObject::from_pairs([
            ("id", JsonValue::str(self.id.clone())),
            ("paired", JsonValue::Array(paired)),
        ]));
        write_atomic(path, doc.js_stringified().as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_keeps_its_id_and_tokens_across_opens() {
        let dir = std::env::temp_dir().join(format!("djnet-store-{}", super::super::proto::random_hex(4)));
        let path = dir.join("djnet.json");
        let mut a = Store::open(Some(path.clone()));
        a.put("peer1", "Pixel", "tok");
        let b = Store::open(Some(path.clone()));
        assert_eq!(a.id, b.id);
        assert_eq!(b.token_of("peer1"), Some("tok"));
        let mut b = b;
        assert_eq!(b.forget("Pixel"), 1);
        assert!(Store::open(Some(path)).token_of("peer1").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
