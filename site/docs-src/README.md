# site/docs-src (data only)

What is left of the retired Go btwsite docs package: the files btwserve (`crates/btwserve`, `src/docs.rs`)
reads at runtime through `-docs-src site/docs-src`:

- `api.md`: the hand-written API reference, served at `/docs/api.md` and `/docs/api` (its `https://beatscript.app`
  is replaced by the server's `-public-url`).
- `assets/docs.css`, `assets/docs.js`: the HTML docs' stylesheet and script, `/docs/assets/*`.

Do not move or rename them without changing btwserve's `-docs-src` in the `btw-rust` `~/services` CMD on pflow.dev
(and in this repo `deploy/deploy.sh` CMD) in the same change.
