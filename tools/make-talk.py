#!/usr/bin/env python3
"""Someone talking the way people do in a call, for examples/onset_eval.rs: phrases of
unscripted speech with pauses between them (a moment, a few seconds, a long while of
listening), and a list of where each phrase's sound starts and ends.

    tools/make-talk.py phrases/ talk.wav talk.txt [seed] [most seconds]

`phrases/`: one voice's phrases, 48 kHz mono 16-bit wav, each as it was recorded: their own
levels are kept (people talk louder and softer), only the whole track is brought to -25 dBFS
over its speech. The list has a line per phrase: the samples its sound starts and ends at
(the first and last 5 ms within 30 dB of the phrase's loud parts, with more to follow: not a
click), its level against the track's (dB), and its file. Needs numpy.

Tried with the EARS dataset's unscripted phrases (48 kHz, recorded up close in an anechoic
room: huggingface.co/datasets/NathanRoll/ears-naturalistic-phrases, CC BY-NC 4.0: for
measuring here, never for shipping).
"""
import os
import sys
import wave

import numpy as np

HOP = 240  # 5 ms


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


def levels(x):
    n = len(x) // HOP
    e = (x[: n * HOP].reshape(n, HOP) ** 2).mean(axis=1)
    return 10 * np.log10(np.maximum(e, 1e-12))


def sound(x):
    """Where the phrase's sound starts and ends (samples), and its loud parts' level."""
    db = levels(x)
    loud = np.percentile(db, 95)
    over = db > loud - 30
    # (Three of the next four 5 ms too: a word, not a click.)
    going = np.array([over[i] and over[i + 1 : i + 5].sum() >= 3 for i in range(len(over) - 5)])
    back = np.array([over[i] and over[max(i - 4, 0) : i].sum() >= 3 for i in range(5, len(over))])
    if not going.any() or not back.any():
        return None
    return int(np.argmax(going)) * HOP, (5 + len(back) - int(np.argmax(back[::-1]))) * HOP, loud


src, out_wav, out_txt = sys.argv[1:4]
seed = int(sys.argv[4]) if len(sys.argv) > 4 else 1
most = float(sys.argv[5]) if len(sys.argv) > 5 else 900.0
rng = np.random.default_rng(seed)
names = sorted(f for f in os.listdir(src) if f.endswith(".wav"))
rng.shuffle(names)
track, rows, speech = [np.zeros(48000, np.float32)], [], []
at = 48000
for name in names:
    x = read(os.path.join(src, name))
    found = sound(x)
    if found is None:
        continue
    a, b, _ = found
    if at + len(x) > most * 48000:
        break
    rows.append([at + a, at + b, x[a:b], name])
    speech.append(x[a:b])
    track.append(x)
    at += len(x)
    # A moment (the same thought going on), a few seconds, or a long while of listening.
    kind = rng.random()
    gap = rng.uniform(0.15, 0.6) if kind < 0.35 else rng.uniform(0.9, 4.0) if kind < 0.7 else rng.uniform(5.0, 14.0)
    track.append(np.zeros(int(gap * 48000), np.float32))
    at += int(gap * 48000)
track = np.concatenate(track)


def active_db(x):
    db = levels(x)
    return 10 * np.log10(np.mean(10 ** (db[db > db.max() - 30] / 10)))


whole = active_db(np.concatenate(speech))
track *= 10 ** ((-25 - whole) / 20)
write(out_wav, track)
with open(out_txt, "w") as f:
    for a, b, x, name in rows:
        f.write(f"{a} {b} {active_db(x) - whole:.1f} {name}\n")
print(f"{len(rows)} phrases in {len(track) / 48000:.0f} s; levels against the track's: "
      + " ".join(f"{np.percentile([active_db(r[2]) - whole for r in rows], q):+.0f}" for q in (5, 25, 50, 75, 95)) + " dB (5th to 95th percentile)")
