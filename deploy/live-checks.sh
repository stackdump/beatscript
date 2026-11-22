#!/usr/bin/env bash
# deploy/live-checks.sh: every live check a beatscript.app deploy must pass, against the public origin. Run from the
# repo root (deploy/deploy.sh check does). Each line either prints what it saw or stops the script (set -e): exit 0
# means every check passed.
#
#   deploy/live-checks.sh <public url> <version>     e.g. deploy/live-checks.sh https://beatscript.app btwserve-abc1234
#
# The checks accumulated wave by wave in the staged deploy scripts (waves 1-3b, doc comments, 6662/6664/6665/6666/
# 6672/6679/6689/6690); they are copied here verbatim, only the paths changed for this one repo. After them:
# crates/btwserve/deploy/mcp-exposure.py (the MCP exposure table live, 6679), site/deploy/deploy.sh
# verify-beatscript, and the live smoke steps 5, 13 and 14 in headless Chrome (CHROME, default /usr/bin/google-chrome).
#
# A failing check names itself: the ERR trap prints "LIVE CHECK FAILED: line N (exit C): <command>" on stderr, and the
# script exits with that command's code. (The second cutover, 2026-11-22, rolled back on a curl that timed out, exit
# 28, inside `EH=$(curl ...)`: set -e alone exits there without a word, so no check said FAIL.) -E carries the trap
# into the functions below, where a bare set -e exits silently too.
#
# curl retries a transport failure, and only that: no answer at all (exit 6 resolve, 7 connect, 28 timeout, 35 TLS,
# 52 empty reply, 55 send, 56 receive) is tried again up to LIVE_CHECK_RETRIES times (default 3), LIVE_CHECK_RETRY_S
# apart (default 5), which rides out a short network gap on the ops machine's side. Any answer the server gives, of
# any status, is judged exactly as before; a check never passes on a retry that a single good answer would fail.
set -eE
cd "$(dirname "$0")/.."
# (only the main shell says it: a $(...) that fails is reported once, as the command that holds it)
trap 'rc=$?; [ "$BASHPID" != $$ ] || echo "LIVE CHECK FAILED: deploy/live-checks.sh line $LINENO (exit $rc): $BASH_COMMAND" >&2' ERR
# the live smoke writes its screenshots over the tracked site/test/shots/*.png: put them back, so a deploy leaves the
# checkout clean (the next deploy's preflight refuses a dirty tree)
trap 'git checkout -q -- site/test/shots 2>/dev/null; git clean -qf -- site/test/shots >/dev/null 2>&1' EXIT
curl() {
  local t rc n=0
  t=$(mktemp)
  while :; do
    command curl "$@" >"$t" && rc=0 || rc=$?
    case $rc in 6|7|28|35|52|55|56) ;; *) break ;; esac
    [ "$n" -lt "${LIVE_CHECK_RETRIES:-3}" ] || break
    n=$((n + 1))
    echo "live-checks: curl exit $rc, no answer (transport): retry $n of ${LIVE_CHECK_RETRIES:-3} in ${LIVE_CHECK_RETRY_S:-5} s: curl $*" >&2
    sleep "${LIVE_CHECK_RETRY_S:-5}"
  done
  # (`curl ... | grep -q` closes the pipe at its first match: a cat cut short there is not a failed check)
  cat "$t" 2>/dev/null || :
  rm -f "$t"
  return "$rc"
}
U=${1:-https://beatscript.app}
V=${2:?usage: deploy/live-checks.sh <public url> <btwserve version, e.g. btwserve-abc1234>}
for i in $(seq 1 30); do command curl -s -m 5 $U/healthz | grep -q $V && break; sleep 2; done
# (shown on stderr without `tee /dev/stderr`, which reopens a redirected stderr file and truncates the log)
HZ=$(curl -s -m 10 $U/healthz); printf '%s\n' "$HZ" >&2; printf '%s' "$HZ" | grep -q $V
for p in /grammar /grammar.ebnf /.well-known/btw-capabilities.json /docs/cheatsheet /authorship /authorship.md /phrasebook /phrasebook.txt /phrasebook.jsonld /docs/found-phrases /docs/found-phrases.md /docs/forms /docs/forms.md /themes /themes.txt /themes.jsonld /docs/embed /docs/embed.md /embed.js /js/embed-player.js; do
  c=$(curl -s -o /dev/null -m 20 -w '%{http_code}' $U$p); echo "$p $c"; test "$c" = 200
done
for p in /grammar.css /lineage.css /mcp.css /card/card.css /css/btw.css /docs/assets/docs.css /css/embed.css; do
  c=$(curl -s -o /dev/null -m 20 -w '%{http_code} %{content_type}' $U$p); echo "$p $c"
  case "$c" in '200 text/css'*) ;; *) echo "$p: want 200 text/css"; exit 1;; esac
done
for p in / /docs/ /docs/cheatsheet /docs/found-phrases /docs/forms /themes /phrasebook /schema /mcp /grammar /authorship /privacy; do
  if curl -s -m 20 $U$p | grep -qiE '<style| style='; then echo "$p has an inline style"; exit 1; fi
done
echo 'no inline styles on / /docs/ /docs/cheatsheet /docs/found-phrases /docs/forms /themes /phrasebook /schema /mcp /grammar /authorship /privacy'
# --- wave 1 / authorship (from wave2-deploy-beatscript.sh) ---
A=$(curl -s -m 20 $U/authorship)
curl -s -o /dev/null -m 20 -w '%{content_type}' $U/authorship | grep -q '^text/html'; echo '/authorship is text/html'
printf '%s' "$A" | grep -q "prompts alone don't make you its author"; printf '%s' "$A" | grep -q 'href="https://www.copyright.gov/ai/"'; printf '%s' "$A" | grep -q 'This is plain language, not legal advice.'; echo '/authorship: the approved text, its source and the disclaimer'
curl -s -m 20 $U/sitemap.xml | grep -q "<loc>$U/authorship</loc>"; echo 'sitemap lists /authorship'
curl -s -m 20 $U/llms.txt | grep -q "$U/authorship"; echo 'llms.txt lists /authorship'
for p in / /docs/ /grammar /schema /mcp /ontology/ /privacy /delete-account; do curl -s -m 20 $U$p | grep -q 'href="/authorship"' || { echo "$p: no link to /authorship"; exit 1; }; done
echo 'the footer links /authorship on / /docs/ /grammar /schema /mcp /ontology/ /privacy /delete-account'
C=$(curl -s -m 20 $U/sitemap.xml | grep -o '/p/z[1-9A-HJ-NP-Za-km-z]*' | head -1)
if [ -n "$C" ]; then curl -s -m 20 $U$C.jsonld | grep -q "\"usageInfo\": \"$U/authorship\""; echo "$C.jsonld: usageInfo -> /authorship"; else echo 'no stored public program in the sitemap: usageInfo not checked live'; fi
mcp() { curl -s -m 30 -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' -d "$1" $U/mcp; }
T=$(mcp '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}')
for n in listen generate expand push_private genre_template share list_examples; do printf '%s' "$T" | grep -q "\"name\":\"$n\""; echo "mcp tool $n ok"; done
printf '%s' "$T" | grep -q "plain language: $U/authorship"; echo 'mcp share points to /authorship'
curl -s -m 20 $U/privacy | grep -q 'in a small language called BeatScript'; echo '/privacy: a small language called BeatScript'
curl -s -m 20 $U/mcp | grep -q 'claude mcp add --transport http beatscript '; echo '/mcp: the server is named beatscript'
curl -s -m 20 $U/js/rights.js | grep -q "href: AUTHORSHIP_PATH"; echo 'rights.js served (the share dialog line)'
curl -s -m 20 $U/js/editor.js | grep -q "this.body.style.setProperty('--lh'"; echo 'editor.js keeps the gutter fix'
curl -s -m 10 $U/ontology/health | grep -q '^ok'; echo 'ontology/health ok'
curl -s -m 10 $U/ | grep -q 'id="lnk-docs"'; echo 'home has lnk-docs'
curl -s -m 20 $U/grammar.ebnf | grep -q '"drive"'; echo 'grammar.ebnf: drive'
curl -s -m 20 $U/grammar.ebnf | grep -q 'lyric_text'; echo 'grammar.ebnf: lyric_text'
curl -s -m 20 $U/docs/cheatsheet.md | grep -q 'drive'; echo 'cheat sheet teaches drive'
printf '%s' "$T" | grep -q 'formatted on the way in'; echo 'mcp descriptions: formatted on the way in'
# --- wave 2 (from wave2-deploy-beatscript.sh) ---
G=$(curl -s -m 20 $U/grammar.ebnf)
printf '%s' "$G" | grep -q '"never"'; echo 'grammar.ebnf: assert never (6598)'
printf '%s' "$G" | grep -q '"launch"'; echo 'grammar.ebnf: launch (6600)'
printf '%s' "$G" | grep -q '"template"'; echo 'grammar.ebnf: template (6642)'
printf '%s' "$G" | grep -q '\[ INT \] mut_target'; echo 'grammar.ebnf: ~N / @N (6640)'
printf '%s' "$G" | grep -q 'ROMAN \[ "/" INT \]'; echo 'grammar.ebnf: roman numerals in calls (6596)'
curl -s -m 20 $U/docs/cheatsheet.md | grep -q 'rests('; echo 'cheat sheet teaches rests() (6599)'
PB=$(curl -s -m 20 $U/phrasebook.jsonld | grep -o '"@id": *"z4EBG9j[^"]*"' | head -1 | grep -o 'z4EBG9j[^"]*')
if [ -n "$PB" ]; then c=$(curl -s -o /dev/null -m 20 -w '%{http_code}' $U/phrasebook/$PB.jsonld); echo "/phrasebook/$PB.jsonld $c"; test "$c" = 200; else echo 'no corpus CID found in /phrasebook.jsonld (not checked)'; fi
curl -s -m 20 $U/llms.txt | grep -q "$U/phrasebook"; echo 'llms.txt lists /phrasebook'
curl -s -m 20 $U/api/examples | python3 -c 'import json,sys; d=json.load(sys.stdin); d=d.get("examples",d) if isinstance(d,dict) else d; assert d[0]["name"]=="arrange-in-eight-lines", d[0]["name"]; assert d[0].get("cid","z4EBG9jELuNXCvY9tz2z1dCY3JwD13P9cRb1D9nKUjAE58MWVTz")=="z4EBG9jELuNXCvY9tz2z1dCY3JwD13P9cRb1D9nKUjAE58MWVTz"'; echo 'api/examples: arrange-in-eight-lines first, CID as pinned'
curl -s -m 20 $U/examples/arrange-in-eight-lines.btw | grep -q 'One section, played eight times'; echo 'arrange-in-eight-lines.btw keeps its teaching comments'
chk() { curl -s -m 30 -H 'Content-Type: application/json' -d "$(python3 -c 'import json,sys; print(json.dumps({"text": sys.argv[1]}))' "$1")" $U/api/check | python3 -c 'import json,sys; d=json.load(sys.stdin); assert d["ok"] is True, d["diagnostics"]'; }
chk 'arrange from template(deep-house, @tempest_such_stuff)'; echo 'live check: template(deep-house, @tempest_such_stuff) ok'
chk 'tempo 100
let call = x ~ x ~
deck A: snare call
deck B: clap rests(call)'; echo 'live check: rests() ok'
chk 'tempo 120
key am
let syn_arp = a3 c4 e4 a4 g4 e4 c4 e4
section s 2 bars {
  deck B: arp syn_arp inst=bright-pluck
}
arrange {
  s
  ~2 syn_arp jux rev
  s
} loop 4'; echo 'live check: ~N + jux ok'
# --- wave 3, found phrases (from found-phrases-deploy-beatscript.sh) ---
curl -s -o /dev/null -m 20 -w '%{content_type}' $U/docs/found-phrases | grep -q '^text/html'; echo '/docs/found-phrases is text/html'
curl -s -m 20 $U/docs/found-phrases.md | grep -q '^# Found phrases: borrowing text for its shape'; echo '/docs/found-phrases.md is the technique'
curl -s -m 20 $U/sitemap.xml | grep -q "<loc>$U/docs/found-phrases</loc>"; echo 'sitemap lists /docs/found-phrases'
curl -s -m 20 $U/llms.txt | grep -q "$U/docs/found-phrases.md"; echo 'llms.txt lists found-phrases'
curl -s -m 20 $U/docs/ | grep -q 'href="/docs/found-phrases"'; echo 'the docs index lists found-phrases'
curl -s -m 20 $U/docs/cheatsheet.md | grep -q 'found-phrases.md'; echo 'the cheat sheet links found-phrases'
curl -s -m 20 $U/docs/BTW.md | grep -q 'found-phrases.md'; echo 'BTW.md links found-phrases'
test "$(curl -s -m 20 $U/docs/cheatsheet.md | wc -c)" -le 30000; echo 'the cheat sheet is within its 30 KB cap'
I=$(mcp '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"deploy-check","version":"1"}}}')
printf '%s' "$I" | grep -q 'consider a phrasebook line chosen for its meter'; echo 'mcp instructions: the authoring loop has the phrasebook step'
printf '%s' "$I" | grep -q 'consider a classical form'; echo 'mcp instructions: the authoring loop has the forms step'
printf '%s' "$T" | grep -q '"phrasebook"'; echo 'mcp: list_examples takes phrasebook'
printf '%s' "$T" | grep -q 'found-phrases'; echo 'mcp: the descriptions point at found-phrases'
L=$(mcp '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_examples","arguments":{"meter":"trochaic"}}}')
printf '%s' "$L" | grep -q 'tyger_burning_bright'; echo 'mcp list_examples meter=trochaic lists the Tyger'
P=$(mcp '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"genre_template","arguments":{"genre":"techno","phrase":"tyger_burning_bright"}}}')
printf '%s' "$P" | grep -q 'sing @tyger_burning_bright on melody'; echo 'mcp genre_template techno phrase=tyger_burning_bright sings it'
# --- wave 3b, forms and themes (from form-templates-deploy-beatscript.sh) ---
curl -s -m 20 $U/docs/forms.md | grep -q '^# Forms: arrange shapes from classical music'; echo '/docs/forms.md is the forms doc'
for c in z4EBG9j9Z3mPn1vKYrEVc93WXEjYyL7bCGdBM7jX4h8ZWMEuYZx z4EBG9j7BJKjYPfs6QQBQCdjieviWbSwbJqEPm9idg79wTYNLW1 z4EBG9izUsbisU2JZtPHUo4hcQpYd8FLyZXLqKUmbtftjj3Lirw; do curl -s -m 20 $U/docs/forms.md | grep -q $c; done; echo '/docs/forms.md names the three reference programs'
curl -s -m 20 $U/sitemap.xml | grep -q "<loc>$U/docs/forms</loc>"; curl -s -m 20 $U/sitemap.xml | grep -q "<loc>$U/themes</loc>"; echo 'sitemap lists /docs/forms and /themes'
curl -s -m 20 $U/llms.txt | grep -q "$U/docs/forms.md"; curl -s -m 20 $U/llms.txt | grep -q "$U/themes.txt"; echo 'llms.txt lists forms and themes'
curl -s -m 20 $U/docs/ | grep -q 'href="/docs/forms"'; curl -s -m 20 $U/docs/ | grep -q 'href="/themes"'; echo 'the docs index lists forms and themes'
curl -s -m 20 $U/docs/cheatsheet.md | grep -q 'docs/forms.md'; echo 'the cheat sheet has the forms line'
curl -s -m 20 $U/docs/BTW.md | grep -q 'forms.md'; echo 'BTW.md links forms'
TC=$(curl -s -m 20 $U/themes.jsonld | python3 -c 'import json,sys; print(json.load(sys.stdin)["@id"])')
test "$(curl -s -o /dev/null -m 20 -w '%{http_code}' $U/themes/$TC.txt)" = 200; echo "/themes/$TC.txt by CID"
curl -s -m 20 $U/themes.txt | grep -q '^ode_to_joy | melody | d | major'; echo '/themes.txt is the theme library'
printf '%s' "$T" | grep -q 'docs/forms.md'; echo 'mcp: the descriptions point at forms'
printf '%s' "$T" | grep -q '"theme"'; printf '%s' "$T" | grep -q '"composer"'; echo 'mcp: genre_template takes theme, list_examples takes composer'
X=$(mcp '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_examples","arguments":{"composer":"bach"}}}')
printf '%s' "$X" | grep -q 'bach_prelude_c'; echo 'mcp list_examples composer=bach lists the Prelude in C'
F=$(mcp '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"genre_template","arguments":{"genre":"house","form":"rondo"}}}')
printf '%s' "$F" | grep -q 'arrange from template(rondo, @shanty_drunken_sailor)'; printf '%s' "$F" | grep -q 'rag-rondo'; echo 'mcp genre_template house form=rondo is the rondo, with its reference'
H=$(mcp '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"genre_template","arguments":{"genre":"techno","theme":"mountain_king"}}}')
printf '%s' "$H" | grep -q 'let mountain_king_1 = '; echo 'mcp genre_template techno theme=mountain_king puts the theme in'
# genre_template works live at all (7c90af27 installs the genre graph at startup; before it, live answered "not available")
GT=$(mcp '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"genre_template","arguments":{"genre":"techno"}}}')
printf '%s' "$GT" | grep -q '#! genre: techno'; if printf '%s' "$GT" | grep -q 'not available'; then echo 'genre_template: genre conventions not available'; exit 1; fi; echo 'mcp genre_template techno works live'
chk 'arrange from template(rondo, @hands_up)'; echo 'live check: template(rondo, @hands_up) ok'
chk 'arrange from template(passacaglia, @tyger_burning_bright)'; echo 'live check: template(passacaglia, @tyger_burning_bright) ok'
# --- doc comments (project-datum 6662) ---
DOCP='tempo 120
# drop: the kick and the bass
section drop 1 bars {
  deck A: kick x ~ x ~
}
arrange drop'
BARE='tempo 120
section drop 1 bars {
  deck A: kick x ~ x ~
}
arrange drop'
ck() { curl -s -m 30 -H 'Content-Type: application/json' -d "$(python3 -c 'import json,sys; print(json.dumps({"text": sys.argv[1]}))' "$1")" $U/api/check; }
CD=$(ck "$DOCP"); CB=$(ck "$BARE")
python3 -c '
import json,sys
d=json.loads(sys.argv[1]); b=json.loads(sys.argv[2])
assert d["ok"] is True and b["ok"] is True, (d.get("diagnostics"), b.get("diagnostics"))
assert d["sections"][0].get("doc") == "the kick and the bass", d["sections"]
assert d.get("docs") == [{"kind": "section", "name": "drop", "doc": "the kick and the bass"}], d.get("docs")
assert "doc" not in b["sections"][0] and "docs" not in b, b
assert d["cid"] == b["cid"] and d["canonical"] == b["canonical"], (d["cid"], b["cid"])
print("live check: sections[0].doc and docs for a commented program; one CID with and without the comment:", d["cid"])
' "$CD" "$CB"
MC=$(mcp "$(python3 -c 'import json,sys; print(json.dumps({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"check","arguments":{"text":sys.argv[1]}}}))' "$DOCP")")
printf '%s' "$MC" | grep -q 'the kick and the bass'; echo 'mcp check says the doc comment'
curl -s -m 20 $U/schema | grep -q 'id="section"'; echo '/schema defines section'
curl -s -m 20 $U/docs/BTW.md | grep -q 'Doc comments.'; echo 'BTW.md has the doc-comments paragraph'
# --- 6672: genre style for trap (drums-808 kit, every chord of a bar) ---
TRAPDRUMS='deck A: kick x ~ ~ ~ ~ ~ x ~ ~ ~ x ~ ~ ~ ~ ~ kit=drums-808 | snare ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~ kit=drums-808 | hihat x*16 kit=drums-808'
gck() { curl -s -m 30 -H 'Content-Type: application/json' -d "$(python3 -c 'import json,sys; print(json.dumps({"text": sys.argv[1], "genre": sys.argv[2]}))' "$1" "$2")" $U/api/check; }
for PAD in '[e3, g3, b3] ~ ~ ~ ~ ~ ~ ~ [c3, e3, g3] ~ ~ ~ ~ ~ ~ ~' '[e3,g3,b3] ~ ~ ~ [d3,f#3,a3] ~ ~ ~ [c3,e3,g3] ~ ~ ~ [d3,f#3,a3] ~ ~ ~'; do
  gck "tempo 140
key em
$TRAPDRUMS
deck B: harmony $PAD" trap | python3 -c '
import json,sys
d=json.load(sys.stdin); s=d.get("style") or {}
assert d["ok"] is True, d.get("diagnostics")
assert s.get("genre") == "https://ontology.bitwrap.io/genre/trap", s
w=s.get("warnings")
assert w == [], w
print("live check: genre=trap, kit=drums-808, pad", sys.argv[1], ": no style warning")' "$PAD"
done
gck 'tempo 140
deck A: kick x ~ x ~ kit=drums-cr78' trap | python3 -c '
import json,sys
w=json.load(sys.stdin)["style"]["warnings"]
assert [x["code"] for x in w] == ["kit"] and "(its kit is drums-808)" in w[0]["message"], w
print("live check: kit=drums-cr78 is still unusual for trap (its kit is drums-808)")'
TM=$(mcp "$(python3 -c 'import json,sys; print(json.dumps({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"check","arguments":{"text":sys.argv[1],"genre":"trap"}}}))' "tempo 140
key em
$TRAPDRUMS")")
printf '%s' "$TM" | grep -q 'genre/trap'; if printf '%s' "$TM" | grep -q 'is unusual for trap'; then echo 'mcp check genre=trap: still a kit warning'; exit 1; fi; echo 'mcp check genre=trap on a drums-808 program: no kit warning'
GTT=$(mcp '{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"genre_template","arguments":{"genre":"trap"}}}')
printf '%s' "$GTT" | grep -q 'kit=drums-808'; echo 'mcp genre_template trap uses kit=drums-808'
# --- 6664: the embed player ---
EC=z4EBG9jELuNXCvY9tz2z1dCY3JwD13P9cRb1D9nKUjAE58MWVTz   # arrange-in-eight-lines, the first example (pinned above)
EH=$(curl -s -D - -o /dev/null -m 20 $U/embed/$EC)
printf '%s' "$EH" | head -1 | grep -q ' 200'; echo "/embed/$EC 200"
printf '%s' "$EH" | grep -i '^content-security-policy:' | grep -q 'frame-ancestors \*'; echo '/embed/: frame-ancestors *'
if printf '%s' "$EH" | grep -qi '^x-frame-options:'; then echo '/embed/ still sends X-Frame-Options'; exit 1; fi; echo '/embed/: no X-Frame-Options'
curl -s -D - -o /dev/null -m 20 $U/ | grep -i '^content-security-policy:' | grep -q "frame-ancestors 'none'"; echo "/: frame-ancestors 'none' still"
EP=$(curl -s -m 20 $U/embed/$EC)
printf '%s' "$EP" | grep -q 'id="bsp-play"'; printf '%s' "$EP" | grep -q 'src="/js/embed-player.js"'; printf '%s' "$EP" | grep -q '<beatscript-view'; echo '/embed/: the player, its script and the program block'
if printf '%s' "$EP" | grep -qiE '<style| style='; then echo '/embed/ has an inline style'; exit 1; fi; echo '/embed/: no inline style'
test "$(curl -s -o /dev/null -m 20 -w '%{http_code}' $U/embed/zNotAProgramCid1111111111111111111111111111)" = 404; echo '/embed/<unknown> 404'
curl -s -m 20 "$U/oembed?url=$U/c/$EC&format=json" | python3 -c '
import json,sys
d=json.load(sys.stdin); c=sys.argv[1]
assert d["version"] == "1.0" and d["type"] == "rich", d
assert "/embed/" + c in d["html"] and "<iframe" in d["html"], d["html"]
assert d["width"] and d["height"] and d.get("provider_name"), d
print("/oembed for /c/" + c + ": rich, the iframe of /embed/")' $EC
curl -s -D - -o /dev/null -m 20 -H 'Origin: https://example.com' $U/embed.js | grep -iq '^access-control-allow-origin: \*'; echo '/embed.js from another origin: ACAO *'
curl -s -m 20 $U/embed.js | grep -q 'beatscript-player'; echo '/embed.js defines beatscript-player'
curl -s -m 20 $U/docs/embed.md | grep -q '^# Embed a program'; echo '/docs/embed.md is the embed doc'
curl -s -m 20 $U/sitemap.xml | grep -q "<loc>$U/docs/embed</loc>"; echo 'sitemap lists /docs/embed'
curl -s -m 20 $U/llms.txt | grep -q "$U/docs/embed"; echo 'llms.txt lists /docs/embed'
curl -s -m 20 $U/c/$EC | grep -q 'application/json+oembed'; curl -s -m 20 $U/c/$EC | grep -q 'name="twitter:player"'; echo "/c/$EC: oEmbed discovery and the player card"
printf '%s' "$(mcp '{"jsonrpc":"2.0","id":11,"method":"tools/list","params":{}}')" | grep -q 'embed_url'; echo 'mcp share names embed_url'
# --- 6665/6666: the lexicon harvest ---
LX=$(mcp "$(python3 -c 'import json,sys; print(json.dumps({"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"check","arguments":{"text":sys.argv[1]}}}))' 'tempo 120
key am
deck B: melody c4 e4 g4 e4 | harmony chord(am)
deck B: sing "jingle=/j ii ng g uh l/ bells" on melody')")
printf '%s' "$LX" | python3 -c '
import json,sys
r=json.load(sys.stdin)["result"]; d=json.loads(r["content"][0]["text"])
assert d["ok"] is True, d.get("diagnostics")
assert isinstance(d.get("lexicon"), dict) and "harvested" in d["lexicon"] and "new" in d["lexicon"], d.get("lexicon")
assert "jingle" not in json.dumps(d["diagnostics"]), d["diagnostics"]
print("mcp check: jingle=/j ii ng g uh l/ checks clean, and the answer carries lexicon", d["lexicon"])'
mcp '{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"list_examples","arguments":{"lexicon":true}}}' | python3 -c '
import json,sys
r=json.load(sys.stdin)["result"]; assert not r.get("isError"), r
d=json.loads(r["content"][0]["text"]); assert isinstance(d["lexicon"]["words"], list), d.get("lexicon")
print("mcp list_examples lexicon=true answers", len(d["lexicon"]["words"]), "harvested words")'
# --- 6679: every shipped feature described and reachable through MCP, live (the table cargo test runs) ---
python3 -I crates/btwserve/deploy/mcp-exposure.py $U
site/deploy/deploy.sh verify-beatscript
# live smoke: 5 = share (incl. the rights line linking /authorship), 13 = caret on the glyph, 14 = gutter rows
CHROME=${CHROME:-/usr/bin/google-chrome} BTW_CHROME_ARGS=--disable-audio-output node site/test/smoke.mjs --server $U --only 5,13,14
# --- 6689/6690 reference programs ---
curl -s -m 20 $U/docs/found-phrases.md | grep -q such-stuff-as-dreams-122-reference.btw; echo /docs/found-phrases.md names the such-stuff reference file
curl -s -m 20 $U/docs/found-phrases.md | grep -q examples/btw/tyger-tyger-128.btw; echo /docs/found-phrases.md names the tyger reference file
echo "LIVE CHECKS PASSED: $V at $U"
