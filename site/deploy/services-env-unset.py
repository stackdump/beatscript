#!/usr/bin/env python3
"""Patch ~/services on the host so `restart` stops leaking variables deleted from an env file.

Why: ~/services sources ~/.env-services/<name> into the service's long-lived tmux window shell with
`set -a; . file; set +a`. That adds and overwrites but never unsets, so a variable removed from the file stayed
exported and the next start inherited it. BTW_ASSETLINKS_FILE did exactly that: it was removed from
~/.env-services/btw-bitwrap and the restarted beatscript-serve still served the old one-fingerprint file
(2026-11-26). The patched send_env first unsets the names it exported last time (kept in
~/.env-services/.keys-<name>: variable NAMES only, mode 0600, never values), then sources the file.

Run on the host (site/deploy/deploy.sh services-env-unset pipes it over ssh). Idempotent; backs up first; bash -n
before it replaces anything, restores the backup if that fails.
"""
import os, shutil, subprocess, sys, time

p = os.path.expanduser("~/services")
s = open(p).read()
if ".keys-" in s:
    print("services-env-unset: already patched"); sys.exit(0)

old = '''send_env() {
    local name="$1"
    if [ -f "$ENV_DIR/$name" ]; then
        tmux send-keys -t $SESSION:"$name" "set -a; . $ENV_DIR/$name; set +a" Enter
    fi
}'''
new = '''send_env() {
    local name="$1"
    # The window shell outlives restarts and `set -a; . file` only adds or overwrites, so a variable deleted
    # from the env file stayed exported. Unset the names exported last time first; the key file holds variable
    # NAMES only, never values.
    local keys="$ENV_DIR/.keys-$name"
    if [ -f "$ENV_DIR/$name" ]; then
        tmux send-keys -t $SESSION:"$name" "[ -f $keys ] && unset \\$(cat $keys); set -a; . $ENV_DIR/$name; set +a" Enter
        sed -n -E 's/^(export +)?([A-Za-z_][A-Za-z0-9_]*)=.*/\\2/p' "$ENV_DIR/$name" > "$keys"
        chmod 600 "$keys"
    fi
}'''
if old not in s:
    print("services-env-unset: send_env is not in the expected shape; refusing", file=sys.stderr); sys.exit(1)

bak = f"{p}.bak-env-unset-{int(time.time())}"
shutil.copy2(p, bak)
tmp = p + ".new"
open(tmp, "w").write(s.replace(old, new))
os.chmod(tmp, os.stat(p).st_mode)
if subprocess.run(["bash", "-n", tmp]).returncode != 0:
    os.remove(tmp); print(f"services-env-unset: bash -n failed, nothing changed (backup {bak})", file=sys.stderr); sys.exit(1)
os.replace(tmp, p)
print(f"services-env-unset: patched ~/services (backup {bak}); takes effect on the next `restart <name>`")
