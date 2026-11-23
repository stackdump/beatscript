// The program a fresh editor opens with. It is BeatScript 2, as every text the page writes: its version line comes first.

import { LINE } from './version.js';

export const STARTER = `${LINE}
# a first program: drums on deck A, harmony and an arpeggio on deck B.
tempo 126
key am

deck A: kick e(5,8) | snare ~ x ~ x | bass markov(am, 0.7) | hihat x*8 vel=60..90
deck B: harmony chord(am7) | arp arp(am7, up, 8)

on phrase: deck A: filter lp 12k -> 400 over 4 bars
master: filter lp 2k -> 8k over 8 bars
`;
