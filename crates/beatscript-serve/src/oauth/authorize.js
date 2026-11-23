// The OAuth consent page (beatscript-serve src/oauth.rs): sign in with the site's own sign-in (/js/auth.js, the same
// challenge and message as the page's), then send the decision and follow the redirect it answers.
const status = document.getElementById('status');
const say = (m, cls) => { status.textContent = m; status.className = cls || ''; };

async function signin(kind) {
  try {
    const [{ signIn }, { makeApi }] = await Promise.all([import('/js/auth.js'), import('/js/api.js')]);
    say('Signing in…');
    await signIn(makeApi(), kind, { hosts: [location.host] });
    location.reload();
  } catch (e) {
    say('Sign-in failed: ' + ((e && e.message) || e), 'err');
  }
}

async function decide(btn, approve) {
  for (const b of document.querySelectorAll('button')) b.disabled = true;
  say(approve ? 'Connecting…' : 'Declining…');
  try {
    const r = await fetch('/oauth/authorize', {
      method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ req: btn.dataset.req, consent: btn.dataset.consent, approve }),
    });
    const j = await r.json();
    if (!r.ok || !j.redirect) throw new Error(j.error_description || j.error || r.status);
    say(approve ? 'Allowed. Returning to the app…' : 'Declined. Returning to the app…', 'ok');
    location.assign(j.redirect);
  } catch (e) {
    say('Could not finish: ' + ((e && e.message) || e), 'err');
    for (const b of document.querySelectorAll('button')) b.disabled = false;
  }
}

const $ = (id) => document.getElementById(id);
if ($('approve')) $('approve').addEventListener('click', () => decide($('approve'), true));
if ($('deny')) $('deny').addEventListener('click', () => decide($('deny'), false));
if ($('signin-key')) $('signin-key').addEventListener('click', () => signin('ed25519'));
if ($('signin-eth')) {
  if (globalThis.ethereum && globalThis.ethereum.request) $('signin-eth').hidden = false;
  $('signin-eth').addEventListener('click', () => signin('eth'));
}
