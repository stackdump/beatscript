# site/embed: the embed clients

The code every BeatScript embed shares, and the clients for hosts that are not a web page. What each host gets, and how
to use it, is [docs/beatscript/embed.md](../../docs/beatscript/embed.md) (served at https://beatscript.app/docs/embed).

| file | what it is |
|---|---|
| `embed-config.js` | the embed config: `normalize`, `fromElement`, `toAttributes`, `artifactCapabilities`, `detectHost` and `resolve`. The contract is `site/web/embed-config.schema.json`; `SCHEMA` here is the same object. The player and the studio packages carry this file |
| `jupyter/beatscript_embed.py` | a notebook helper: `Track(...)`, `Studio(...)` and `show(...)` make the cell's output the player or the studio. One file, the standard library only |
| `jupyter/test_beatscript_embed.py` | its test: `python3 -I site/embed/jupyter/test_beatscript_embed.py` |

These files may name `https://beatscript.app`: the site's own scripts (`site/web/js/`) may not
(`site/scripts/beatscript-host-literals.sh`).

`node --test site/test/embed-config.test.mjs` tests `embed-config.js`. beatscript-serve mirrors `normalize`, `toAttributes` and
`artifactCapabilities` in Rust, held to the same fixtures.
