"""beatscript_embed: the HTML's shape, the text in its attribute, the static fallback link, the pins held to the server's lock files,
and that nothing touches the network. Standard library only:

    python3 -I site/embed/jupyter/test_beatscript_embed.py
"""

import html
import importlib
import json
import os
import socket
import sys
import types
import unittest
import urllib.request
from html.parser import HTMLParser
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
LOCKS = os.path.join(HERE, '..', '..', '..', 'crates', 'beatscript-serve', 'src', 'mcp')
sys.path.insert(0, HERE)  # -I leaves the script's own directory off sys.path

import beatscript_embed as be  # noqa: E402

CID = 'zb2rhe5P4gXftAwvA4eXQ5HJwsER2owDyS9sKaQRRVQPn93bA'
TEXT = '#! name: two bars\ntempo 120\ndeck A: kick x ~ x ~ | snare ~ x ~ x\n'
HASHES = {'beatscript-track.js': 'sha384-ENTRY', 'js/wasm.js': 'sha384-WASM', 'beatscript-track.umd.js': 'sha384-UMD'}


class Tree(HTMLParser):
    """Every element as (tag, attrs, depth), and each element's own text, as a browser's tokenizer splits them."""

    def __init__(self, s):
        super().__init__(convert_charrefs=True)
        self.nodes, self.data, self.open = [], {}, []
        self.feed(s)
        self.close()

    def handle_starttag(self, tag, attrs):
        self.open.append(len(self.nodes))
        self.nodes.append((tag, dict(attrs), len(self.open) - 1))

    def handle_endtag(self, tag):
        self.open.pop()

    def handle_data(self, d):
        if self.open:
            self.data[self.open[-1]] = self.data.get(self.open[-1], '') + d

    def find(self, tag, **want):
        return [(i, a, d) for i, (t, a, d) in enumerate(self.nodes) if t == tag and all(a.get(k) == v for k, v in want.items())]


def with_hashes(pkg, hashes):
    return mock.patch.dict(pkg, {'integrity': dict(hashes)})


class Shape(unittest.TestCase):
    def test_player_tags_in_order(self):
        with with_hashes(be.PLAYER, HASHES):
            page = be.Track(text=TEXT, cid=CID)._repr_html_()
        t = Tree(page)
        order = [n[0] for n in t.nodes]
        self.assertEqual(order, ['script', 'script', 'beatscript-track', 'a'])
        imap = json.loads(t.data[0])
        base = f"https://cdn.jsdelivr.net/npm/@stackdump/beatscript-player@{be.PLAYER['version']}/"
        # the entry's hash is on its tag; the import map has every other module, not the UMD wrapper
        self.assertEqual(imap, {'integrity': {base + 'js/wasm.js': 'sha384-WASM'}})
        self.assertEqual(t.nodes[0][1], {'type': 'importmap'})
        self.assertEqual(t.nodes[1][1], {'type': 'module', 'src': base + 'beatscript-track.js', 'integrity': 'sha384-ENTRY', 'crossorigin': 'anonymous'})
        self.assertEqual(list(t.nodes[2][1]), ['cid', 'text'])
        self.assertEqual(t.nodes[2][1], {'cid': CID, 'text': TEXT})

    def test_tags_are_artifact_rs(self):
        """The default tags are PlayerCdn::tags of the lock, byte for byte: the same files, URLs, order and escaping."""
        got = lock('player-cdn.lock.json')
        if got is None:
            self.skipTest('player-cdn.lock.json is not in this checkout')
        url = lambda f: f"https://cdn.jsdelivr.net/npm/{got['package']}@{got['version']}/{f}"
        mods = {url(f): x['integrity'] for f, x in got['files'].items() if f.endswith('.js') and f != got['entry'] and not f.endswith('.umd.js')}
        want = ('<script type="importmap">' + json.dumps({'integrity': mods}, separators=(',', ':')).replace('<', '\\u003c') + '</script>\n'
                f'<script type="module" src="{url(got["entry"])}" integrity="{got["files"][got["entry"]]["integrity"]}" crossorigin="anonymous"></script>\n')
        self.assertGreaterEqual(len(mods), 10)
        self.assertEqual('\n'.join(be.Track(text=TEXT).tags()) + '\n', want)

    def test_no_integrity_when_empty(self):
        with with_hashes(be.PLAYER, {}):
            page = be.Track(text=TEXT).html()
        self.assertNotIn('integrity', page)
        self.assertNotIn('importmap', page)
        self.assertIn(f'<script type="module" src="https://cdn.jsdelivr.net/npm/@stackdump/beatscript-player@{be.PLAYER["version"]}/beatscript-track.js" crossorigin="anonymous"></script>', page)

    def test_studio(self):
        page = be.Studio(text=TEXT, audio='render', theme='light', compact=True, capabilities=['mcp']).html()
        t = Tree(page)
        [mod] = [a for (tag, a, _) in t.nodes if tag == 'script' and a.get('type') == 'module']
        self.assertEqual(mod['src'], 'https://cdn.jsdelivr.net/npm/@stackdump/beatscript-studio@0.1.0/beatscript-studio.js')
        self.assertEqual(mod['integrity'], be.STUDIO['integrity']['beatscript-studio.js'])
        [(_, a, _)] = t.find('beatscript-studio')
        self.assertEqual(list(a), ['text', 'audio', 'theme', 'compact', 'config'])
        self.assertEqual((a['audio'], a['theme'], a['compact']), ('render', 'light', None))
        self.assertEqual(json.loads(a['config']), {'capabilities': ['mcp']})

    def test_player_attributes_in_order(self):
        el = be.Track(text='tempo 120\n', cid=CID, mode='render', features=['highlight', 'share'], theme='light', compact=True,
                      site='https://example.org', title="T's").attributes()
        self.assertEqual(el, f' cid="{CID}" text="tempo 120\n" mode="render" highlight theme="light" compact site="https://example.org"'
                             ' config="{&quot;features&quot;:[&quot;highlight&quot;,&quot;share&quot;],&quot;title&quot;:&quot;T&#39;s&quot;}"')

    def test_pin(self):
        with with_hashes(be.PLAYER, HASHES):
            other = be.Track(text=TEXT, pin={'version': '9.9.9'}).html()
            off = be.Track(text=TEXT, pin={'sri': False}).html()
        self.assertIn('beatscript-player@9.9.9/beatscript-track.js', other)
        self.assertNotIn('integrity', other)  # the lock's hashes are for the pinned version only
        self.assertNotIn('integrity', off)
        with self.assertRaises(ValueError):
            be.Track(text=TEXT, pin={'version': 'latest'}).html()

    def test_refuses_unknown_keys(self):
        with self.assertRaises(TypeError):
            be.Track(text=TEXT, colour='red')
        with self.assertRaises(TypeError):
            be.Track()
        with self.assertRaises(TypeError):  # the player would play the fallback link's words
            be.Track(cid=CID)
        self.assertNotIn(' text=', be.Studio(cid=CID).attributes())  # the studio reads no text from its light DOM
        with self.assertRaises(ValueError):
            be.show(TEXT, kind='card')


class Escaping(unittest.TestCase):
    def test_text_round_trips_in_its_attribute(self):
        """The text attribute, unescaped as a browser does, is the text byte for byte: nothing in it is changed, and
        nothing in it can end the attribute or the element."""
        evil = 'tempo 120\n# </script><img src=x onerror=alert(1)> </SCRIPT >\n# <!-- <script> "double" \'single\' & &amp; &#39;\n'
        for cls in (be.Track, be.Studio):
            page = cls(text=evil, cid=CID).html()
            t = Tree(page)
            self.assertEqual([n[0] for n in t.nodes][-2:], [cls(text='x')._tag, 'a'])
            [(_, a, _)] = t.find(cls(text='x')._tag)
            self.assertEqual(a['text'], evil)
            self.assertEqual(t.find('img'), [])
            value = page[page.index(' text="') + 7:]
            value = value[:value.index('"')]
            self.assertEqual(html.unescape(value), evil)
            self.assertNotRegex(value, '[<>\'"]')
            for want in ('&lt;/script&gt;', '&lt;!--', '&quot;double&quot;', '&#39;single&#39;', '&amp;amp;', '&amp;#39;'):
                self.assertIn(want, value)

    def test_attributes_are_escaped(self):
        title = '"><script>alert(1)</script>&'
        page = be.Track(text=TEXT, title=title, site='https://e.org/"x').html()
        t = Tree(page)
        self.assertEqual(len(t.find('script')), 2)  # the import map and the module, nothing injected
        [(_, a, _)] = t.find('beatscript-track')
        self.assertEqual(json.loads(a['config'])['title'], title)
        self.assertEqual(a['site'], 'https://e.org/"x')
        [(i, link, _)] = t.find('a')
        self.assertEqual(t.data[i], f'{title} on e.org')

    def test_import_map_json_cannot_close_the_script(self):
        with with_hashes(be.PLAYER, {'js/</script>.js': 'sha384-x'}):
            page = be.Track(text=TEXT).html()
        self.assertEqual(page.lower().count('</script'), 2)  # the import map and the module


class Fallback(unittest.TestCase):
    def test_link_in_the_light_dom(self):
        t = Tree(be.Track(text=TEXT, cid=CID).html())
        [(_, el, depth)] = t.find('beatscript-track')
        [(i, a, adepth)] = t.find('a')
        self.assertEqual(a, {'href': f'https://beatscript.app/p/{CID}'})
        self.assertEqual(adepth, depth + 1)  # a child of the element, shown wherever the scripts are not run
        self.assertEqual(t.data[i], 'two bars on beatscript.app')

    def test_link_without_a_cid_and_another_site(self):
        t = Tree(be.Studio(text='tempo 90\n', site='https://example.org/').html())
        [(i, a, _)] = t.find('a')
        self.assertEqual(a['href'], 'https://example.org/')
        self.assertEqual(t.data[i], 'a BeatScript program on example.org')

    def test_plain_repr(self):
        self.assertEqual(repr(be.Track(text=TEXT, cid=CID)), f'<Track: two bars https://beatscript.app/p/{CID}>')


class NoNetwork(unittest.TestCase):
    def test_import_render_and_show_fetch_nothing(self):
        used = []

        def refuse(*a, **k):
            used.append(a)
            raise OSError('the network was used')

        shown = []
        ipython = types.ModuleType('IPython')
        ipython.display = types.ModuleType('IPython.display')
        ipython.display.display = shown.append
        with mock.patch.object(socket.socket, 'connect', refuse), mock.patch.object(socket, 'create_connection', refuse), \
                mock.patch.object(socket, 'getaddrinfo', refuse), mock.patch.object(urllib.request, 'urlopen', refuse), \
                mock.patch.dict(sys.modules, {'IPython': ipython, 'IPython.display': ipython.display}):
            sys.modules.pop('beatscript_embed', None)
            fresh = importlib.import_module('beatscript_embed')
            fresh.Track(text=TEXT, cid=CID)._repr_html_()
            fresh.Studio(text=TEXT)._repr_html_()
            fresh.show(TEXT, kind='studio')
        self.assertEqual(used, [])
        self.assertEqual([type(e).__name__ for e in shown], ['Studio'])


def lock(name):
    path = os.path.join(LOCKS, name)
    if not os.path.exists(path):
        return None
    with open(path, encoding='utf-8') as f:
        return json.load(f)


class Locks(unittest.TestCase):
    """The pins are the server's: the same package, version and entry, and the hashes empty or exactly the lock's."""

    def check(self, name, pinned):
        got = lock(name)
        if got is None:
            self.skipTest(f'{name} is not in this checkout')
        self.assertEqual((pinned['package'], pinned['version'], pinned['entry']), (got['package'], got['version'], got['entry']))
        if got.get('placeholder'):
            # a placeholder hash would stop the package from running
            self.assertEqual(pinned['integrity'], {}, f'{name} is a placeholder: leave the integrity empty')
        else:
            self.assertEqual(pinned['integrity'], {f: x['integrity'] for f, x in got['files'].items()})

    def test_player(self):
        self.check('player-cdn.lock.json', be.PLAYER)

    def test_studio(self):
        self.check('studio-cdn.lock.json', be.STUDIO)


if __name__ == '__main__':
    unittest.main(verbosity=1)
