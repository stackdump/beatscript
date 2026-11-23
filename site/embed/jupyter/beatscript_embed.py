"""BeatScript in a notebook: a program and its player, or the whole studio, as a cell's output.

    from beatscript_embed import Track, Studio, show
    Track(text=open('song.btw').read())              # a cell's last expression shows the player
    Studio(text=text, theme='light')                 # the editor, its checker and the player
    show(text, kind='studio')                        # display it from anywhere in a cell

One file and the standard library: copy it beside the notebook, nothing to install. Nothing is fetched when it is
imported or when it makes its HTML; the browser fetches the package when it shows the output. The HTML is the tags the
web embed uses (crates/btwserve/src/mcp/artifact.rs): an import map giving every module of the package its Subresource
Integrity hash, the entry as a module script from jsDelivr at an exact version, then the element with the program in its
text attribute, escaped as artifact.rs's esc does. The attribute gives the element the text byte for byte, where a
child <script>'s escapes would change it (and so its CID).

Where a notebook does not run its outputs' scripts (an untrusted notebook, a static viewer such as GitHub's), the
element's light DOM shows instead: a plain link to the program, <site>/p/<CID> when a cid is given. The element hides
it once it has loaded (its card is in a shadow root).
"""

import json
import re
from urllib.parse import urlsplit

# The packages, each pinned to one exact version. integrity maps a file of the package to its SRI hash, exactly as the
# lock files give them (crates/btwserve/src/mcp/player-cdn.lock.json and studio-cdn.lock.json; test_beatscript_embed.py
# holds them to those files). Empty, the HTML carries no integrity at all rather than wrong hashes, which would stop the
# package from running.
PLAYER = {'package': '@stackdump/beatscript-player', 'version': '0.1.0', 'entry': 'beatscript-track.js', 'integrity': {
    'beatscript-track.js': 'sha384-mDuTIvWvwff8EJ6jMEEZmFxj/tQuEvwUaWSry/8EXZlfZ8g9evMpt36h8bPm5Xgt',
    'beatscript-track.umd.js': 'sha384-BFmeZkF5eDDEJzIfDCl8jbuvI8dnnNeijcPQFsD800deWrcgwJbiLoI5GZrlzB96',
    'js/embed-model.js': 'sha384-2bMFrpO2YPKzlfzdHUR4OD3DKxm49b12NifeekjL4t2fCa5so0si3re5fb+ptBIg',
    'js/engine-calls.js': 'sha384-s5o2VhqWXteZlCABYoF4YMYLGgPRzXqnTUKnKp58mKlCo7GVyxFAiMSpHPyni1sl',
    'js/engine-composer.js': 'sha384-4JudpWCLec+hX/C73cgqfb8laXQvGT0dVMs2MMKYNWPYlWijFLp/Bmc2e3h+jJak',
    'js/engine-polyfill.js': 'sha384-xTDNQuv0GC4j63Q03ZWL3EePH80NHM4aq9mDsunL+SREkLGQRVtUKlIF0WsjASTc',
    'js/engine-worklet.js': 'sha384-SX3qcRQw68p6yuCFYr4ww45TPXWQPCCrotU5x3UvguQ6QMDDfcJ88pRAOUkFO1nN',
    'js/engine.js': 'sha384-e5+tdz5vRgiz+1Who2ie+PfqDAOMWz5P62W9scTFScAo3ZxtmCZoKqJShpvJBKW5',
    'js/mixer-model.js': 'sha384-3VluHJuwCDuhaotOqq616HSWXpyfHrtrl20e/aeatY4YXKX70sRonCntsCbhvQpk',
    'js/wasm-mock.js': 'sha384-UYwxLE5icoXMnhWEvylWHq6TfND9y6ImAiv3WpauY39LuvoN+r6ErBhn4VjOpqBC',
    'js/wasm.js': 'sha384-Fnnbr22SvXtJyjAFqGoYJUyFCjWppkQVbtW9eAKk47ZfZbThXvWxZqUQQhN4uIuN',
    'wasm/btw.js': 'sha384-JIYt9zz7HhfCBkT7z2zJe6sXbY93yhz+44vcMtEvKi288SnpGGqiQGTxv2ddBEWu',
    'wasm/btw_bg.wasm.b64.js': 'sha384-ETinehDel81+MkpG5jfanpzsCmomTsMkaTqzykvLltghBL9mFbqnYerGzEUqSBVl',
}}
STUDIO = {'package': '@stackdump/beatscript-studio', 'version': '0.1.0', 'entry': 'beatscript-studio.js', 'integrity': {
    'beatscript-render.js': 'sha384-e+hk6c8Q83+nTbuvY8YbIvccw4QDD8rxmCKp8ofLY9adSx31NmTBn1QDOg2+YEf7',
    'beatscript-studio.js': 'sha384-t+4Mp4Y1Ypspd1XEkAu8CU9frVpnQDn7ejsl7NO9eb9rovgKuAv6rVkUznjafP9i',
    'beatscript-track.js': 'sha384-42GLGoCS+q+OCJHS1EAvP+Aju6fa6tXqk00XxlaGDvz/m7YDhjG/hx65NFh4QHCa',
    'beatscript-track.umd.js': 'sha384-BFmeZkF5eDDEJzIfDCl8jbuvI8dnnNeijcPQFsD800deWrcgwJbiLoI5GZrlzB96',
    'embed-config.js': 'sha384-b2l2DiDA2L5j6yI5W1BrLxrUqaCs6RjA/8pofoV3llnmDdPFpOkbk3aR5dlMHaWS',
    'js/editor.js': 'sha384-IPSQttsZEeKbNBREehZT3jYYkwkzJE22Md3QGfDvBq2xHl4vWFjEM1FZr9z4UMbj',
    'js/embed-model.js': 'sha384-2bMFrpO2YPKzlfzdHUR4OD3DKxm49b12NifeekjL4t2fCa5so0si3re5fb+ptBIg',
    'js/engine-calls.js': 'sha384-s5o2VhqWXteZlCABYoF4YMYLGgPRzXqnTUKnKp58mKlCo7GVyxFAiMSpHPyni1sl',
    'js/engine-composer.js': 'sha384-4JudpWCLec+hX/C73cgqfb8laXQvGT0dVMs2MMKYNWPYlWijFLp/Bmc2e3h+jJak',
    'js/engine-polyfill.js': 'sha384-xTDNQuv0GC4j63Q03ZWL3EePH80NHM4aq9mDsunL+SREkLGQRVtUKlIF0WsjASTc',
    'js/engine-worklet.js': 'sha384-SX3qcRQw68p6yuCFYr4ww45TPXWQPCCrotU5x3UvguQ6QMDDfcJ88pRAOUkFO1nN',
    'js/engine.js': 'sha384-DXzsnw0ni2EFWwCpQZi1cEy/xXIWPwny6yVh8WcB1J4wFVYzWWNkIXaIWO96vNQy',
    'js/geometry.js': 'sha384-t920UjuaFC12EP61lVpQMDksX0NOyJAxfilFt+bhCc/xumULUBeBip2v35U+YAZ5',
    'js/highlight.js': 'sha384-wlLt8/Y+pgDlprN8g+xZ6uyLy/pOuitewIeqVFbqfVeWp8Th51sKdYqBR8MKXmJE',
    'js/mixer-model.js': 'sha384-3VluHJuwCDuhaotOqq616HSWXpyfHrtrl20e/aeatY4YXKX70sRonCntsCbhvQpk',
    'js/station-model.js': 'sha384-6cS4dBJcsHvmq/i5kIMI6kURkLhhC7+r/w4TVzmKoA1w8jR/KxMKWeaE+jOWR3rB',
    'js/util.js': 'sha384-zYrpi3cOv3yd6Hm6rI1j6IM31IEak45/wr1JRKp2Fch2vJvmJmWJxXXfUD5l1lf+',
    'js/wasm-mock.js': 'sha384-OZP0jK+ud23D9kGCgNfllzd63m7eA2r2EJAdGG29rCXRfBDgeXu2ktfyGAA7lVxN',
    'js/wasm.js': 'sha384-GAZ558TToW7uUO/pXBlRKj53DTD47A9h3lLFVHe5KfcthCco4+IuswMGEbjxWtkZ',
    'js/wrap-model.js': 'sha384-J+zcIE5KOthBEsD0myALU3ZX8QiyAkP0/i6Z7oB8VICNISB8+Zq3px9gHJHYfpVS',
    'render-model.js': 'sha384-bVdsMMmDmn6vM+kJgSj2Vf93OIUcbWRNBD5fYaWtR+I/B7d/swKVFV9ZortzOpli',
    'render-worker.js': 'sha384-k0qWTAhBC0n274OS2zj5dn36dB1xbREgiah495ux4yVJ10lseW+D1RgnrcfRq22f',
    'studio-css.js': 'sha384-Ro1XQxH5PHCFUkwz0ZB4cW3g1kxa11sZ1PBU82UHYRTNsuR/xAPhzjuCpEZkaoWT',
    'tool-surface.js': 'sha384-JrvpgwdbeyqfsljYWcId124QO0BQRiRXxnEP27jCifEraaKQ/lKI3se4NQDErUTQ',
    'wasm/btw.js': 'sha384-JIYt9zz7HhfCBkT7z2zJe6sXbY93yhz+44vcMtEvKi288SnpGGqiQGTxv2ddBEWu',
    'wasm/btw_bg.wasm.b64.js': 'sha384-3WNzDfwvNBY4H+YOIZzQpGlD3gyD96UOLA4off4f6VkuYj2FLKw24P3yofxtKRDp',
}}

CDN = 'https://cdn.jsdelivr.net/npm/'
DEFAULT_SITE = 'https://beatscript.app'

# The config's keys (site/web/embed-config.schema.json), and the two attribute names the elements also read: mode
# (audio's alias) and highlight (adds the highlight feature).
CONFIG_KEYS = ('v', 'source', 'audio', 'features', 'capabilities', 'theme', 'compact', 'width', 'height', 'title', 'site',
               'bridge', 'pin', 'mode', 'highlight')
VERSION = re.compile(r'[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?')


_ESC = {'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'}


def attr(s):
    """HTML text and attribute escaping: the five characters artifact.rs's esc changes, as it changes them."""
    return re.sub('[&<>"\']', lambda m: _ESC[m.group()], str(s))


def name_of(text):
    """`#! name:` of a program, else ''."""
    m = re.search(r'^#!\s*name:\s*(.+)$', text or '', re.M)
    return m.group(1).strip() if m else ''


class _Embed:
    _package = None  # PLAYER or STUDIO
    _tag = None
    _audio_attr = None  # the player reads mode=, the studio audio=
    _needs_text = False

    def __init__(self, text=None, cid=None, **config):
        unknown = sorted(set(config) - set(CONFIG_KEYS))
        if unknown:
            raise TypeError(f"{type(self).__name__}: no config key {', '.join(unknown)}; the keys are {', '.join(CONFIG_KEYS)}")
        if text is None and self._needs_text:
            # without a text attribute the player plays its element's own text, which is the fallback link
            raise TypeError(f'{type(self).__name__}: the player plays a text; give text (a cid is the CID it must have)')
        if text is None and cid is None and 'source' not in config:
            raise TypeError(f'{type(self).__name__}: give the program as text, a cid or a source')
        self.text, self.cid, self.config = text, cid, dict(config)

    def package(self):
        """The package to load: the pinned one, or pin.version's without hashes (the lock has none for another version);
        pin.sri false gives no hashes either."""
        pkg = dict(self._package)
        pin = self.config.get('pin') or {}
        version = pin.get('version', pkg['version'])
        if not VERSION.fullmatch(str(version)):
            raise ValueError(f'pin.version {version!r} is not an exact version such as 0.2.0')
        if version != pkg['version'] or pin.get('sri') is False:
            pkg['version'], pkg['integrity'] = version, {}
        return pkg

    def link(self):
        site = str(self.config.get('site') or DEFAULT_SITE).rstrip('/')
        return f'{site}/p/{self.cid}' if self.cid else f'{site}/'

    def title(self):
        return self.config.get('title') or name_of(self.text) or 'a BeatScript program'

    def tags(self):
        """The import map (only when there are hashes) and the entry's module script, as the web embed writes them."""
        pkg = self.package()
        url = lambda f: f"{CDN}{pkg['package']}@{pkg['version']}/{f}"
        hashes = pkg['integrity']
        # every module the entry can import: each .js but the entry itself and the UMD wrapper (artifact.rs modules())
        modules = {url(f): h for f, h in sorted(hashes.items()) if f.endswith('.js') and f != pkg['entry'] and not f.endswith('.umd.js')}
        out = []
        if modules:
            out.append('<script type="importmap">' + json.dumps({'integrity': modules}, separators=(',', ':')).replace('<', '\\u003c') + '</script>')
        sri = hashes.get(pkg['entry'])
        out.append(f'<script type="module" src="{attr(url(pkg["entry"]))}"' + (f' integrity="{attr(sri)}"' if sri else '') + ' crossorigin="anonymous"></script>')
        return out

    def attributes(self):
        """The element's attributes: cid and text, as artifact.rs writes them, then in toAttributes' order
        (site/embed/embed-config.js) mode or audio, highlight, theme, compact, site, and config, the JSON of every other
        key given. features goes in config whole, and also as the highlight attribute, which is what the published
        player reads."""
        rest = {k: v for k, v in self.config.items() if k not in ('pin', 'mode', 'highlight')}
        audio = rest.pop('audio', self.config.get('mode'))
        pairs = [('cid', self.cid), ('text', self.text), (self._audio_attr, audio)]
        pairs.append(('highlight', True if self.config.get('highlight') or 'highlight' in (rest.get('features') or ()) else None))
        # the published player styles theme="light" only: "auto" stays in config, where the element resolves it
        pairs.append(('theme', rest.pop('theme') if rest.get('theme') in ('light', 'dark') else None))
        pairs.append(('compact', True if rest.pop('compact', False) else None))
        pairs.append(('site', rest.pop('site', None)))
        pairs.append(('config', json.dumps(rest, separators=(',', ':'), ensure_ascii=False) if rest else None))
        return ''.join(f' {k}' if v is True else f' {k}="{attr(v)}"' for k, v in pairs if v is not None)

    def html(self):
        host = urlsplit(self.link()).netloc
        out = self.tags()
        out.append(f'<{self._tag}{self.attributes()}>')
        out.append(f'<a href="{attr(self.link())}">{attr(self.title())} on {attr(host)}</a>')
        out.append(f'</{self._tag}>')
        return '\n'.join(out)

    def _repr_html_(self):
        return self.html()

    def __repr__(self):
        # what a front end without HTML shows: where to hear it
        return f'<{type(self).__name__}: {self.title()} {self.link()}>'


class Track(_Embed):
    """A program and its player, <beatscript-track>. Track(text=..., cid=..., **config), config as embed-config."""
    _package, _tag, _audio_attr, _needs_text = PLAYER, 'beatscript-track', 'mode', True


class Studio(_Embed):
    """The studio, <beatscript-studio>: the editor, its checker and the player. Studio(text=..., **config)."""
    _package, _tag, _audio_attr = STUDIO, 'beatscript-studio', 'audio'


def show(text, kind='player', **config):
    """Display the program now, as the player (kind 'player') or the studio (kind 'studio'). Outside IPython it prints
    the HTML, for a page."""
    kinds = {'player': Track, 'studio': Studio}
    if kind not in kinds:
        raise ValueError(f"kind {kind!r}: a notebook shows 'player' or 'studio'")
    embed = kinds[kind](text=text, **config)
    try:
        from IPython.display import display  # here, not at the top: importing this module needs only the standard library
    except ImportError:
        print(embed.html())
    else:
        display(embed)
