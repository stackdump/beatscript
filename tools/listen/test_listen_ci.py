#!/usr/bin/env python3
"""listen_ci's judgement, without btwd: `python3 tools/listen/test_listen_ci.py`."""
import copy
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import listen_ci as lc  # noqa: E402

BASE = {
    "version": "listen/1", "cid": "zA",
    "master": {"lufs": -12.0, "clipped_samples": 0},
    "detected": {"bpm": 124, "bpm_match": "same", "declared_bpm": 124, "key": "A minor", "key_match": "same"},
    "decks": {"A": {"lufs": -14.0, "silent": False}, "B": {"lufs": -15.0, "silent": False}},
    "parts": [{"deck": "B", "part": "bass", "measured": True, "lufs": -15.0, "silent": False}],
    "silent": [],
    "sections": [{"name": "intro", "from_bar": 1, "lufs": -12.5}],
}


def now(**edit):
    v = copy.deepcopy(BASE)
    for path, val in edit.items():
        o = v
        keys = path.split("__")
        for k in keys[:-1]:
            o = o[int(k)] if isinstance(o, list) else o[k]
        o[keys[-1]] = val
    return v


class Compare(unittest.TestCase):
    def test_the_same_passes(self):
        self.assertEqual(lc.compare(BASE, now()), ("pass", []))

    def test_a_deck_that_goes_silent_fails_loudly(self):
        v = now(decks__B__silent=True, decks__B__lufs=None)
        v["silent"] = [{"deck": "B", "part": None, "section": None, "reason": "crossfader"}]
        st, why = lc.compare(BASE, v)
        self.assertEqual(st, "fail")
        self.assertTrue(any(w.startswith("SILENT now: deck B") and "crossfader" in w for w in why), why)

    def test_loudness_drift_past_tolerance(self):
        self.assertEqual(lc.compare(BASE, now(master__lufs=-12.9))[0], "pass")
        st, why = lc.compare(BASE, now(master__lufs=-13.5))
        self.assertEqual(st, "fail")
        self.assertIn("master", why[0])
        self.assertEqual(lc.compare(BASE, now(sections__0__lufs=-10.0))[0], "fail")
        self.assertEqual(lc.compare(BASE, now(parts__0__lufs=-20.0))[0], "fail")

    def test_tempo_and_key(self):
        self.assertEqual(lc.compare(BASE, now(detected__bpm_match="half", detected__bpm=62))[0], "fail")
        self.assertEqual(lc.compare(BASE, now(detected__key="C major"))[0], "fail")

    def test_clipping_starts(self):
        self.assertEqual(lc.compare(BASE, now(master__clipped_samples=12))[0], "fail")

    def test_a_changed_program_is_stale(self):
        self.assertEqual(lc.compare(BASE, now(cid="zB"))[0], "stale")
        self.assertEqual(lc.compare(BASE, now(version="listen/2"))[0], "stale")


class Expects(unittest.TestCase):
    def test_expect_lines(self):
        here = os.path.join(lc.FIXTURES, "silent-crossfader.btw")
        exp = lc.expects(here)
        self.assertIn(("silent", {"deck": "B", "reason": "crossfader"}), exp)
        heard = {"decks": {"A": {"silent": False}, "B": {"silent": True}},
                 "silent": [{"deck": "B", "part": None, "section": None, "reason": "crossfader", "why": "x"}]}
        self.assertEqual(lc.check_expects(heard, exp), [])
        # the bug comes back unnoticed by listen: deck B heard
        lost = {"decks": {"A": {"silent": False}, "B": {"silent": False}}, "silent": []}
        self.assertTrue(lc.check_expects(lost, exp))
        # the right deck, the wrong reason
        wrong = copy.deepcopy(heard)
        wrong["silent"][0]["reason"] = "mute"
        self.assertTrue(lc.check_expects(wrong, exp))


if __name__ == "__main__":
    unittest.main()
