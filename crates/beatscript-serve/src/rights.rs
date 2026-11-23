//! Who authored a program and what rights it has: one text (docs/AUTHORSHIP.md), many views. The text is served as
//! `/authorship` (HTML, docs.rs) and `/authorship.md`; every HTML page the server writes ends with [`FOOTER`], each
//! stored program's JSON-LD names the page as its `usageInfo`, and the share dialogs (web and Android), the MCP share
//! tool and llms.txt link it in one line. None of them repeats the text.

/// The text, built in (as the schema page is), so every install answers `/authorship`.
pub const AUTHORSHIP_MD: &str = include_str!("../../../docs/AUTHORSHIP.md");

/// The page's path, as every view links it.
pub const PATH: &str = "/authorship";

/// Where the code lives (MIT; programs and content are CC0): the footer, /authorship, llms.txt, the MCP instructions
/// and the embed's rights line name it.
pub const SOURCE_URL: &str = "https://github.com/stackdump/beatscript";

/// The footer every HTML page the server writes ends with: a link to [`PATH`], one to [`SOURCE_URL`] and the licences. djlive's Grammar page writes the same
/// bytes (it cannot see this crate); a test holds them equal.
pub const FOOTER: &str = "<footer class=\"rights\"><a href=\"/authorship\">Authorship and rights</a> &middot; <a href=\"https://github.com/stackdump/beatscript\">source</a> &middot; Code MIT &middot; programs CC0</footer>\n";

/// The page's URL on `origin` (`https://beatscript.app`).
pub fn url(origin: &str) -> String {
    format!("{}{PATH}", origin.trim_end_matches('/'))
}

/// `footer.rights`, for each stylesheet a page links (none may be inline: the CSP is `style-src 'self'`).
pub const FOOTER_CSS: &str = "footer.rights{margin:2rem auto 1.5rem;padding:0 1rem;max-width:72rem;font-size:.85rem;opacity:.8}\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_footer_links_the_page() {
        assert!(FOOTER.contains(&format!("href=\"{PATH}\"")));
        assert!(FOOTER.contains(&format!("href=\"{SOURCE_URL}\"")) && FOOTER.contains("Code MIT"));
        assert_eq!(url("https://beatscript.app/"), "https://beatscript.app/authorship");
        // djlive's Grammar page cannot use this constant: it writes the same footer
        assert!(djlive::grammar::page::grammar_page("https://beatscript.app").contains(FOOTER), "the Grammar page has no footer link to /authorship");
        assert!(djlive::grammar::page::stylesheet().contains("footer.rights"));
    }
}
