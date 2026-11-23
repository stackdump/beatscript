// The page's preferences in localStorage, under their BeatScript names, each read from the key it had before the rename
// when the new one is not there yet, so no setting is lost across the deploy. (rename-to-beatscript: keep. This file
// spells the old keys on purpose.)
//
//   prefGet(name, dflt)   the new key; else the old key, copied to the new one; else dflt
//   prefSet(name, value)  the new key (the old one is left as it was: a rollback still finds the value it knew)
//   prefRemove(name)      both keys, so a cleared value does not come back from the old one
//
// The account key is not a preference and is not here: localStorage `btw-ed25519-v1` and IndexedDB `btw-keys` keep their
// names for good (auth.js).

/** name -> [the key, the key before the rename] */
export const PREFS = Object.freeze({
  follow: ['beatscript-follow', 'btw-follow'],
  level: ['beatscript-level', 'btw-level'],
  composeMode: ['beatscript.composeMode', 'btw.composeMode'],
  betaEmail: ['beatscript_beta_email', 'btw_beta_email'],
  source: ['beatscript-source', 'btw-source'],
});

const local = () => { try { return globalThis.localStorage || null; } catch { return null; } };

function keys(name) {
  const k = PREFS[name];
  if (!k) throw new Error(`no preference ${name}`);
  return k;
}

export function prefGet(name, dflt = null, storage = local()) {
  const [key, legacy] = keys(name);
  if (!storage) return dflt;
  try {
    const v = storage.getItem(key);
    if (v != null) return v;
    const old = storage.getItem(legacy);
    if (old == null) return dflt;
    try { storage.setItem(key, old); } catch { /* full or private: the old key still answers */ }
    return old;
  } catch {
    return dflt;
  }
}

export function prefSet(name, value, storage = local()) {
  const [key] = keys(name);
  if (!storage) return;
  try { storage.setItem(key, String(value)); } catch { /* private window */ }
}

export function prefRemove(name, storage = local()) {
  if (!storage) return;
  for (const k of keys(name)) {
    try { storage.removeItem(k); } catch { /* private window */ }
  }
}
