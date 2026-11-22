#!/usr/bin/env bash
# deploy/test-host.sh: `deploy/deploy.sh host` (the checkout on the serving host) against scratch directories, with
# HOST=local and a synthetic origin, so it needs no ssh and no network. The case that broke the first cutover
# (2026-11-22): a --no-checkout clone has no index, and the local-changes guard read its every file as a staged
# deletion and refused, then refused again on every rerun.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd -P)
s=$(mktemp -d); trap 'rm -rf "$s"' EXIT
export GIT_CONFIG_GLOBAL=$s/gitconfig GIT_CONFIG_NOSYSTEM=1
git config --global user.name test; git config --global user.email test@example.invalid
git config --global init.defaultBranch main; git config --global commit.gpgsign false; git config --global advice.detachedHead false

# the origin: a repo carrying this deploy.sh, PIN on main and one later commit on main
src=$s/src; mkdir -p "$src/deploy" "$src/site/web"
cp "$here/deploy.sh" "$src/deploy/deploy.sh"
printf 'a\n' > "$src/site/web/index.html"; printf '/site/bin/\n/site/web/wasm/\n' > "$src/.gitignore"
git -C "$src" init -q; git -C "$src" add -A; git -C "$src" commit -q -m pin
PIN=$(git -C "$src" rev-parse HEAD)
printf 'b\n' > "$src/later.txt"; git -C "$src" add -A; git -C "$src" commit -q -m later
git clone -q --bare "$src" "$s/origin.git"
git -C "$src" checkout -q --detach "$PIN"

home=$s/home; mkdir -p "$home/data"
fails=0
run() { HOME=$home HOST=local REPO_URL=$s/origin.git APP_DIR=app DATA_DIR=data PIN=$PIN "$src/deploy/deploy.sh" host >"$s/out" 2>&1; }
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; sed 's/^/     /' "$s/out"; fails=$((fails + 1)); }
whole() { # app is PIN, detached, every tracked file present and unchanged
  [ "$(git -C "$home/app" rev-parse HEAD)" = "$PIN" ] && [ -z "$(git -C "$home/app" status --porcelain --untracked-files=no)" ] &&
    [ -f "$home/app/site/web/index.html" ] && [ -f "$home/app/deploy/deploy.sh" ] && [ ! -e "$home/app/later.txt" ] &&
    ! git -C "$home/app" symbolic-ref -q HEAD >/dev/null
}

# 1. no checkout yet: clone and check out PIN
if run && whole; then ok "fresh: clones and checks out the pin"; else bad "fresh"; fi
# 2. a rerun on that checkout: nothing to refuse
if run && whole; then ok "rerun: idempotent"; else bad "rerun"; fi

# 3. what the failed cutover left: a --no-checkout clone (no index), at main
rm -rf "$home/app"; git clone -q --no-checkout "$s/origin.git" "$home/app"
if run && whole; then ok "no-checkout leftover (HEAD main): checks the pin out"; else bad "no-checkout leftover (HEAD main)"; fi
# 4. the same with PIN = HEAD, where a bare checkout would move nothing and leave the index empty
rm -rf "$home/app"; git clone -q --no-checkout "$s/origin.git" "$home/app"; git -C "$home/app" update-ref --no-deref HEAD "$PIN"
if run && whole; then ok "no-checkout leftover (HEAD = PIN): fills the index"; else bad "no-checkout leftover (HEAD = PIN)"; fi

# 5. never checked out but holding files: refuse, touch nothing
rm -rf "$home/app"; git clone -q --no-checkout "$s/origin.git" "$home/app"; printf 'x\n' > "$home/app/stray"
if ! run && grep -q 'never checked out but holds files' "$s/out" && [ ! -e "$home/app/.git/index" ] && [ -f "$home/app/stray" ]; then
  ok "no index + stray files: refuses"; else bad "no index + stray files"; fi

# 6. a real local change in a checkout: refuse, keep it
rm -rf "$home/app"; run || true; printf 'edited\n' > "$home/app/site/web/index.html"
if ! run && grep -q 'has local changes' "$s/out" && grep -q edited "$home/app/site/web/index.html"; then
  ok "local change: refuses and keeps it"; else bad "local change"; fi
# 7. a deleted tracked file is a local change too
git -C "$home/app" checkout -q -- .; rm "$home/app/site/web/index.html"
if ! run && grep -q 'has local changes' "$s/out"; then ok "deleted tracked file: refuses"; else bad "deleted tracked file"; fi

# 8. no data dir: refuse before cloning anything
rm -rf "$home/app" "$home/data"
if ! run && grep -q 'no data dir' "$s/out" && [ ! -e "$home/app" ]; then ok "no data dir: refuses"; else bad "no data dir"; fi

[ "$fails" = 0 ] && echo "deploy/test-host.sh: all passed" || { echo "deploy/test-host.sh: $fails failed"; exit 1; }
