#!/usr/bin/env python3
"""A TV across the room, as a microphone hears it, for examples/tv_eval.rs: speech with a music
bed 12 dB under it, looped to six minutes (or as many seconds as asked), through a room's impulse
response (its echo).

    tools/make-tv-room.py speech.wav music.wav room.wav tv.wav [seconds]

All 48 kHz mono 16-bit. DeepFilterNet's assets have a room (rir_sim_1001_w11.7_l2.6_h2.5_rt60_0.7919.wav,
rt60 0.79 s). Written at -30 dBFS (tv_eval scales it). Needs numpy and scipy.
"""
import sys
import wave

import numpy as np
from scipy.signal import oaconvolve


def read(p):
    w = wave.open(p)
    assert w.getframerate() == 48000 and w.getnchannels() == 1 and w.getsampwidth() == 2, p
    return np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768


def write(p, x):
    w = wave.open(p, "wb")
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(48000)
    w.writeframes((np.clip(x, -1, 1) * 32767).astype(np.int16).tobytes())


def rms(x):
    return np.sqrt(np.mean(x**2))


speech, music, room = (read(p) for p in sys.argv[1:4])
n = int(sys.argv[5] if len(sys.argv) > 5 else 6 * 60) * 48000
loop = lambda x: np.tile(x, n // len(x) + 1)[:n]
s, m = loop(speech), loop(music)
tv = s + m * (rms(s) / rms(m)) * 10 ** (-12 / 20)
far = oaconvolve(tv, room)[:n]
write(sys.argv[4], far * 10 ** (-30 / 20) / rms(far))
