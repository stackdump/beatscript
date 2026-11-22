//! Rights on user-made objects (docs/LICENSING.md): the **license**, the **visibility**, the
//! **author** and the **lineage** (`forkedFrom`, `credits`) that ride *inside* a track or a set's
//! canonical JSON. Inside, so they are part of what the content ID hashes: change the license
//! and the CID changes; fork the object and the fork carries them.
//!
//! - `license`: `"CC0-1.0"` (the default: public domain dedication), `"CC-BY-4.0"` (reuse with
//!   attribution) or `"all-rights-reserved"`. There is deliberately no "unlicensed": it reads like
//!   the Unlicense, which is public domain, the opposite of what it would mean here.
//! - `visibility`: `"public"` (the default) or `"private"`. A private object is never shared or
//!   uploaded: [`Rights::check_shareable`] is the one gate every share / upload path asks.
//! - `author`: who made it, as a signer identity (the wallet's `did:key:z…` or an `0x…` address;
//!   any non-empty text is kept). When the object is signed, the signature's issuer is the author.
//! - `forkedFrom`: the parent object's CID, when this one is a remix of it.
//! - `credits`: who must be credited. A fork of a CC BY object adds the parent (its CID, author,
//!   license and name); credits are inherited by every later fork and are never dropped, so a
//!   chain of remixes keeps crediting everyone upstream who asked for it.
//!
//! Generated deterministic snapshots (genre + seed + structure) are not covered here: they are
//! CC0 by definition (the wallet's `beats-snapshot.v1`), and discovery claims stay attribution
//! only. This is for what a person makes.
//!
//! Pure: no I/O, no clock; `no_std` + `alloc`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::json::{JsonObject, JsonValue};

/// What others may do with an object.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum License {
    /// Creative Commons Zero 1.0: dedicated to the public domain. The default.
    #[default]
    Cc0,
    /// Creative Commons Attribution 4.0: reuse, remix, even commercially, with credit.
    CcBy4,
    /// No rights granted beyond what the law allows: listen, but do not reuse.
    AllRightsReserved,
}

impl License {
    pub const ALL: [License; 3] = [License::Cc0, License::CcBy4, License::AllRightsReserved];

    /// The id in the JSON (SPDX for the two CC licenses).
    pub fn id(self) -> &'static str {
        match self {
            License::Cc0 => "CC0-1.0",
            License::CcBy4 => "CC-BY-4.0",
            License::AllRightsReserved => "all-rights-reserved",
        }
    }

    /// For people: "CC0 1.0", "CC BY 4.0", "All rights reserved".
    pub fn label(self) -> &'static str {
        match self {
            License::Cc0 => "CC0 1.0",
            License::CcBy4 => "CC BY 4.0",
            License::AllRightsReserved => "All rights reserved",
        }
    }

    /// The legal text, where there is one.
    pub fn url(self) -> Option<&'static str> {
        match self {
            License::Cc0 => Some("https://creativecommons.org/publicdomain/zero/1.0/"),
            License::CcBy4 => Some("https://creativecommons.org/licenses/by/4.0/"),
            License::AllRightsReserved => None,
        }
    }

    /// One line on what it allows.
    pub fn summary(self) -> &'static str {
        match self {
            License::Cc0 => "public domain: anyone may reuse and remix it, no credit needed",
            License::CcBy4 => "anyone may reuse and remix it, crediting the author",
            License::AllRightsReserved => "the author keeps every right: no reuse or remix without permission",
        }
    }

    /// Whether a fork has to credit the author.
    pub fn requires_attribution(self) -> bool {
        self == License::CcBy4
    }

    /// Whether the license grants reuse at all (a fork of one that does not warns).
    pub fn allows_remix(self) -> bool {
        self != License::AllRightsReserved
    }

    /// The id, the label, a URL or a loose spelling ("cc0", "cc-by", "by", "arr", "all rights
    /// reserved"). Never "unlicensed".
    pub fn parse(text: &str) -> Option<License> {
        let t: String = text
            .trim()
            .chars()
            .filter(|c| !matches!(c, ' ' | '-' | '_' | '.'))
            .flat_map(char::to_lowercase)
            .collect();
        match t.as_str() {
            "cc0" | "cc010" | "cc0public" | "publicdomain" | "pd" => Some(License::Cc0),
            "ccby" | "ccby4" | "ccby40" | "by" | "attribution" => Some(License::CcBy4),
            "allrightsreserved" | "arr" | "reserved" | "copyright" => Some(License::AllRightsReserved),
            _ => License::ALL.into_iter().find(|l| l.url() == Some(text.trim())),
        }
    }

    pub fn ids() -> Vec<&'static str> {
        License::ALL.iter().map(|l| l.id()).collect()
    }
}

/// Who may see an object.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Visibility {
    /// Listed, shareable by CID, publishable. The default.
    #[default]
    Public,
    /// Kept on this machine: never shared by CID, never uploaded.
    Private,
}

impl Visibility {
    pub const ALL: [Visibility; 2] = [Visibility::Public, Visibility::Private];

    pub fn id(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Private => "private",
        }
    }

    pub fn parse(text: &str) -> Option<Visibility> {
        match text.trim().to_ascii_lowercase().as_str() {
            "public" | "pub" | "shared" => Some(Visibility::Public),
            "private" | "priv" | "local" => Some(Visibility::Private),
            _ => None,
        }
    }

    pub fn ids() -> Vec<&'static str> {
        Visibility::ALL.iter().map(|v| v.id()).collect()
    }
}

/// One upstream work a fork must credit (a CC BY ancestor).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credit {
    /// The credited object's CID.
    pub cid: String,
    /// Its author (signer identity), when it named one.
    pub author: Option<String>,
    /// Its license when it was forked (CC BY, for a credit a fork had to add).
    pub license: License,
    /// Its name, when it had one.
    pub name: Option<String>,
}

impl Credit {
    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("cid", self.cid.clone().into());
        if let Some(a) = &self.author {
            o.set("author", a.clone().into());
        }
        o.set("license", self.license.id().into());
        if let Some(n) = &self.name {
            o.set("name", n.clone().into());
        }
        JsonValue::Object(o)
    }

    pub fn from_json(v: &JsonValue) -> Result<Credit, String> {
        let cid = text_of(v, "cid").ok_or("credit: a cid")?;
        let license = match v.get("license").and_then(JsonValue::as_str) {
            Some(l) => License::parse(l).ok_or_else(|| format!("credit: license {l:?}: one of {}", License::ids().join(", ")))?,
            None => License::CcBy4,
        };
        Ok(Credit { cid, author: text_of(v, "author"), license, name: text_of(v, "name") })
    }

    /// "by did:key:z6Mk… · “Night Bus” (zQmAb…9xYz), CC BY 4.0".
    pub fn text(&self) -> String {
        let who = self.author.as_deref().unwrap_or("an unnamed author");
        let what = match &self.name {
            Some(n) => format!("“{n}” ({})", short_cid(&self.cid)),
            None => short_cid(&self.cid),
        };
        format!("by {who} · {what}, {}", self.license.label())
    }
}

/// The rights block of an object.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rights {
    pub license: License,
    pub visibility: Visibility,
    pub author: Option<String>,
    /// The parent's CID.
    pub forked_from: Option<String>,
    pub credits: Vec<Credit>,
}

/// What forking answers: the child's rights and anything the person should be told first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fork {
    pub rights: Rights,
    pub warnings: Vec<String>,
}

/// Why a share / upload was refused.
pub const PRIVATE_REFUSAL: &str = "it is private: private tracks and sets are never shared or uploaded (set visibility public first)";

impl Rights {
    /// CC0, public, nobody named, no parent: what a new object gets.
    pub fn new(author: Option<&str>) -> Rights {
        Rights { author: clean(author), ..Rights::default() }
    }

    /// Write the block into an object's canonical JSON. Every field is written (the license
    /// and visibility even at their defaults), so the CID says what the object is licensed as.
    pub fn write_into(&self, o: &mut JsonObject) {
        o.set("license", self.license.id().into());
        o.set("visibility", self.visibility.id().into());
        if let Some(a) = &self.author {
            o.set("author", a.clone().into());
        }
        if let Some(p) = &self.forked_from {
            o.set("forkedFrom", p.clone().into());
        }
        if !self.credits.is_empty() {
            o.set("credits", JsonValue::Array(self.credits.iter().map(Credit::to_json).collect()));
        }
    }

    /// The block, as its own JSON object (for answers).
    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        self.write_into(&mut o);
        o.set("licenseLabel", self.license.label().into());
        if let Some(u) = self.license.url() {
            o.set("licenseUrl", u.into());
        }
        if !self.credits.is_empty() {
            o.set("creditLine", self.credit_line().into());
        }
        JsonValue::Object(o)
    }

    /// Whether `v` carries a rights block at all (an object saved before there was one has none).
    pub fn present_in(v: &JsonValue) -> bool {
        ["license", "visibility", "author", "forkedFrom", "credits"].iter().any(|k| v.get(k).is_some())
    }

    /// Read the block from an object; absent fields are the defaults. A license or visibility
    /// that is not one of the known ids is an error (never guessed).
    pub fn from_json(v: &JsonValue) -> Result<Rights, String> {
        let license = match v.get("license") {
            None | Some(JsonValue::Null) => License::default(),
            Some(JsonValue::String(l)) => License::parse(l)
                .filter(|p| p.id() == l || p.url() == Some(l.as_str()))
                .ok_or_else(|| format!("license {l:?}: one of {}", License::ids().join(", ")))?,
            Some(_) => return Err("license is text".into()),
        };
        let visibility = match v.get("visibility") {
            None | Some(JsonValue::Null) => Visibility::default(),
            Some(JsonValue::String(s)) => Visibility::ALL
                .into_iter()
                .find(|x| x.id() == s)
                .ok_or_else(|| format!("visibility {s:?}: public or private"))?,
            Some(_) => return Err("visibility is text".into()),
        };
        let credits = match v.get("credits") {
            None | Some(JsonValue::Null) => Vec::new(),
            Some(JsonValue::Array(a)) => a
                .iter()
                .enumerate()
                .map(|(i, c)| Credit::from_json(c).map_err(|e| format!("credit {}: {e}", i + 1)))
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err("credits is a list".into()),
        };
        Ok(Rights {
            license,
            visibility,
            author: text_of(v, "author"),
            forked_from: text_of(v, "forkedFrom"),
            credits,
        })
    }

    /// The one gate for sharing by CID and for every upload: a private object is refused.
    pub fn check_shareable(&self) -> Result<(), String> {
        match self.visibility {
            Visibility::Public => Ok(()),
            Visibility::Private => Err(PRIVATE_REFUSAL.into()),
        }
    }

    /// The rights of a fork of an object with these rights and CID (`parent_name`: its name, for
    /// the credit), made by `author`. The fork keeps the parent's license and visibility (the
    /// person may change either after), points at the parent (`forkedFrom`), inherits every
    /// credit, and adds the parent as a credit when it is CC BY. Forking an all-rights-reserved
    /// object is allowed (it may be the person's own) but warns.
    pub fn fork(&self, parent_cid: &str, parent_name: Option<&str>, author: Option<&str>) -> Fork {
        let mut warnings = Vec::new();
        let mut credits = self.credits.clone();
        match self.license {
            License::AllRightsReserved => warnings.push(format!(
                "{} is all rights reserved{}: its author has not granted reuse, so this fork may not be shared or published without their permission",
                short_cid(parent_cid),
                self.author.as_deref().map(|a| format!(" by {a}")).unwrap_or_default()
            )),
            License::CcBy4 => {
                if self.author.is_none() {
                    warnings.push(format!(
                        "{} is CC BY 4.0 but names no author: it is credited by its CID alone",
                        short_cid(parent_cid)
                    ));
                }
                if !credits.iter().any(|c| c.cid == parent_cid) {
                    credits.push(Credit {
                        cid: parent_cid.to_string(),
                        author: self.author.clone(),
                        license: License::CcBy4,
                        name: clean(parent_name),
                    });
                }
            }
            License::Cc0 => {}
        }
        Fork {
            rights: Rights {
                license: self.license,
                visibility: self.visibility,
                author: clean(author),
                forked_from: Some(parent_cid.to_string()),
                credits,
            },
            warnings,
        }
    }

    /// Who must be credited, as one line ("Contains work by …; …"), or "" when nobody.
    pub fn credit_line(&self) -> String {
        if self.credits.is_empty() {
            return String::new();
        }
        let parts: Vec<String> = self.credits.iter().map(Credit::text).collect();
        format!("Contains work {}", parts.join("; "))
    }

    /// "CC0 1.0 · public", "CC BY 4.0 · private · forked from zQmAb…9xYz".
    pub fn text(&self) -> String {
        let mut s = format!("{} · {}", self.license.label(), self.visibility.id());
        if let Some(p) = &self.forked_from {
            s.push_str(&format!(" · forked from {}", short_cid(p)));
        }
        s
    }
}

fn clean(s: Option<&str>) -> Option<String> {
    s.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn text_of(v: &JsonValue, k: &str) -> Option<String> {
    clean(v.get(k).and_then(JsonValue::as_str))
}

/// `z…tdW4cXT` for lists: the tail, since every CID of one kind shares its multibase head.
pub fn short_cid(cid: &str) -> String {
    let n = cid.chars().count();
    if n <= 12 {
        cid.to_string()
    } else {
        format!("{}…{}", cid.chars().take(1).collect::<String>(), cid.chars().skip(n - 8).collect::<String>())
    }
}
