#!/usr/bin/env python3
"""Generate trance-138.live: a 64-bar hand-written trance arrangement for the
djrust live-code language (docs/LIVECODE.md). Every part is one 64-term <...>
alternation, one term per bar; a full-rest bar stands wherever the part is out.

Arrangement bars are 1-based and `at bar N` fires on the same bar N (the first
music bar is program bar 1: checked against the running clock).
"""
import os, sys

REST = "[" + " ".join(["~"] * 16) + "]"
CH = ["am", "f", "c", "g"]                    # 2 bars per chord, 8-bar cycle
ROOT = {"am": "a2", "f": "f2", "c": "c2", "g": "g2"}
ARP = {"am": "a4 c5 e5 a5 e5 c5 a4 c5", "f": "f4 a4 c5 f5 c5 a4 f4 a4",
       "c": "c5 e5 g5 c6 g5 e5 c5 e5", "g": "g4 b4 d5 g5 d5 b4 g4 b4"}

def chord_of(bar):                             # bar is 1-based
    return CH[((bar - 1) % 8) // 2]

KICK = "[x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~]"
HAT = "[~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~]"
CLAP = "[~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~]"

def every(n):
    return "[" + " ".join("x" if i % n == 0 else "~" for i in range(16)) + "]"

def bass(bar):
    r = ROOT[chord_of(bar)]
    return "[" + " ".join("~" if i % 4 == 0 else r for i in range(16)) + "]"

def arp(bar):
    s = ARP[chord_of(bar)]
    return "[" + s + " " + s + "]"

def pad(bar):
    return f"chord({chord_of(bar)})"

MEL = [  # the 8-bar anthem: bars 1-4 as briefed, 5-8 a variation resolving up to a5
    "[e5 ~ ~ ~ ~ ~ d5 ~ c5 ~ ~ ~ ~ ~ ~ ~]",
    "[~ ~ ~ ~ ~ ~ ~ ~ a4 ~ ~ ~ c5 ~ ~ ~]",
    "[g5 ~ ~ ~ ~ ~ e5 ~ c5 ~ ~ ~ ~ ~ ~ ~]",
    "[~ ~ ~ ~ ~ ~ ~ ~ b4 ~ ~ ~ d5 ~ ~ ~]",
    "[e5 ~ ~ ~ ~ ~ g5 ~ e5 ~ ~ ~ ~ ~ ~ ~]",
    "[~ ~ ~ ~ ~ ~ ~ ~ c5 ~ ~ ~ e5 ~ ~ ~]",
    "[g5 ~ ~ ~ ~ ~ e5 ~ d5 ~ ~ ~ ~ ~ ~ ~]",
    "[~ ~ ~ ~ b4 ~ ~ ~ d5 ~ ~ ~ a5 ~ ~ ~]",
]
def mel(bar):
    return MEL[(bar - 1) % 8]

def part(f, active):
    return "<" + " ".join(f(b) if active(b) else (REST if f is not pad else REST)
                          for b in range(1, 65)) + ">"

def rng(*spans):
    return lambda b: any(lo <= b <= hi for lo, hi in spans)

def roll(b):
    if 25 <= b <= 26: return every(4)
    if 27 <= b <= 28: return every(2)
    if 29 <= b <= 31: return every(1)
    return REST

kick = part(lambda b: KICK, rng((1, 16), (33, 64)))
hat = part(lambda b: HAT, rng((1, 31), (33, 64)))
clap = part(lambda b: CLAP, rng((9, 16), (33, 60)))
snare = part(roll, rng((25, 31)))
bs = part(bass, rng((9, 16), (33, 64)))
ar = part(arp, rng((17, 31), (33, 56)))
pd = part(pad, rng((1, 64)))
me = part(mel, rng((17, 31), (33, 56)))

lines = [
    "tempo 138",
    "key am",
    f"deck A: kick {kick} vel=112 | hihat {hat} vel=70..95 | clap {clap} vel=100 | snare {snare} vel=80..110 | bass {bs} inst=sub-bass oct=2 len=1",
    f"deck B: arp {ar} inst=bright-pluck vel=70..95 len=1 | harmony {pd} inst=warm-pad len=16 vel=80 | melody {me} inst=supersaw len=8 vel=105",
    "master: filter lp 400 -> 16k over 16 bars",
    "at bar 25: master: filter lp 16k -> 1.5k over 1 bars",
    "at bar 26: master: filter lp 1.5k -> 18k over 7 bars",
    "at bar 57: master: filter lp 18k -> 800 over 8 bars",
    "at bar 64: move echo_out on A over 1 bars",
    "at bar 64: move echo_out on B over 1 bars",
]
out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "trance-138.live")
open(out, "w").write("\n".join(lines) + "\n")
print(out, len(open(out).read()), "bytes")
