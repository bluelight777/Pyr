#!/usr/bin/env python3
"""Draws Pyr's built-in soundboard sounds, from scratch (no recordings, so no licences to
worry about): 48 kHz stereo WAV files, one per sound, written to the folder given.

    python3 tools/make-sounds.py out/

Everything is seeded, so the same script gives the same files. Needs numpy and scipy.
"""

import os
import sys
import wave

import numpy as np
from scipy import signal

SR = 48_000
RNG = np.random.default_rng(7)


# --------------------------------------------------------------------------- helpers

def secs(d):
    return np.arange(int(d * SR)) / SR


def phase_of(freq):
    """Running phase (in cycles) of a tone whose frequency changes over time."""
    return np.cumsum(freq) / SR


def harmonics(freq, slope=1.0, top=18_000.0, odd=False, phase=None):
    """A band-limited buzz: harmonics of `freq` (an array, Hz) at 1/k**slope, none above
    `top`, so a falling or rising tone never folds back into noise."""
    ph = phase_of(freq) if phase is None else phase
    out = np.zeros_like(ph)
    fmax = float(np.max(freq))
    k = 1
    while k * fmax < top:
        if not odd or k % 2 == 1:
            out += np.sin(2 * np.pi * k * ph) / k**slope
        k += 1
    return out


def sos_filter(x, kind, f, order=2):
    sos = signal.butter(order, f, btype=kind, fs=SR, output="sos")
    return signal.sosfilt(sos, x)


def resonator(x, f, bw):
    """A single resonance (a formant): band-pass around `f` with bandwidth `bw` Hz."""
    lo, hi = max(20.0, f - bw / 2), min(SR / 2 - 100, f + bw / 2)
    return sos_filter(x, "bandpass", [lo, hi], order=2)


def env(n, attack, release, curve=None):
    """Attack and release ramps (seconds) over `n` samples, times an optional curve."""
    e = np.ones(n)
    a, r = int(attack * SR), int(release * SR)
    if a > 0:
        e[:a] *= np.linspace(0, 1, a) ** 1.5
    if r > 0:
        e[-r:] *= np.linspace(1, 0, r) ** 1.5
    return e if curve is None else e * curve


def decay(n, tau):
    return np.exp(-np.arange(n) / SR / tau)


def place(track, x, at):
    """Mix mono or stereo `x` into `track` (stereo, 2×N) starting at `at` seconds."""
    i = int(at * SR)
    if x.ndim == 1:
        x = np.stack([x, x])
    # Nothing may be cut off (that clicks): the track has to be long enough.
    assert i + x.shape[1] <= track.shape[1], f"{(i + x.shape[1]) / SR:.2f} s doesn't fit in {track.shape[1] / SR:.2f} s"
    track[:, i : i + x.shape[1]] += x


def pan(x, p):
    """Mono to stereo, `p` from -1 (left) to 1 (right), constant power."""
    a = (p + 1) * np.pi / 4
    return np.stack([x * np.cos(a), x * np.sin(a)])


def room(track, wet=0.18, length=0.9, bright=6000.0):
    """A soft room: a stereo tail of decaying noise, convolved."""
    n = int(length * SR)
    t = np.arange(n) / SR
    tails = []
    for _ in range(2):
        ir = RNG.standard_normal(n) * np.exp(-t / (length / 5))
        ir = sos_filter(ir, "lowpass", bright)
        ir[: int(0.012 * SR)] = 0  # pre-delay
        ir /= np.sqrt(np.sum(ir**2))
        tails.append(ir)
    out = np.zeros((2, track.shape[1] + n - 1))
    for c in range(2):
        out[c] = signal.fftconvolve(track[c], tails[c])
        out[c, : track.shape[1]] = track[c] * (1 - wet) + out[c, : track.shape[1]] * wet
        out[c, track.shape[1]:] *= wet
    return out


def finish(track, loud_db=-14.0, peak_db=-1.0, fade=0.03):
    """Make it as loud as the others (by its loudest 0.4 s), keep peaks below `peak_db`,
    trim silence at the end and fade the edges."""
    mono = track.mean(axis=0)
    w = int(0.4 * SR)
    power = np.convolve(mono**2, np.ones(w) / w, mode="valid") if len(mono) > w else np.array([np.mean(mono**2)])
    loud = 10 * np.log10(max(power.max(), 1e-12))
    track = track * 10 ** ((loud_db - loud) / 20)
    limit = 10 ** (peak_db / 20)
    track = np.tanh(track / limit) * limit
    # Trim the tail once it's below -50 dB.
    alive = np.where(np.abs(track).max(axis=0) > 10 ** (-50 / 20))[0]
    if len(alive):
        track = track[:, : alive[-1] + int(0.05 * SR)]
    f = int(fade * SR)
    track[:, :f] *= np.linspace(0, 1, f)
    track[:, -f:] *= np.linspace(1, 0, f)
    return track


def stereo(d):
    return np.zeros((2, int(d * SR)))


# --------------------------------------------------------------------------- the sounds

def quack():
    def one(f0, f1, d):
        n = int(d * SR)
        t = np.arange(n) / SR
        f = np.linspace(f0, f1, n) * (1 + 0.025 * np.sin(2 * np.pi * 31 * t))
        src = harmonics(f, slope=0.55, top=9000.0) + 0.1 * RNG.standard_normal(n)
        voice = (
            1.0 * resonator(src, 1050, 260)
            + 0.9 * resonator(src, 1750, 320)
            + 0.45 * resonator(src, 2900, 500)
            + 0.25 * resonator(src, 600, 200)
        )
        # The "q": a short rasp before the vowel.
        click = sos_filter(RNG.standard_normal(n), "bandpass", [1500, 4500]) * decay(n, 0.012) * 0.6
        shape = np.minimum(1, t / 0.012) * (0.55 + 0.45 * np.exp(-t / 0.09)) * np.minimum(1, (d - t) / 0.05)
        return sos_filter(np.tanh(2.2 * (voice + click) * shape), "lowpass", 6000)

    tr = stereo(1.1)
    place(tr, pan(one(330, 250, 0.26), -0.1), 0.02)
    place(tr, pan(one(300, 230, 0.30), 0.1), 0.40)
    return finish(room(tr, wet=0.10, length=0.4))


def airhorn():
    def blast(d):
        n = int(d * SR)
        t = np.arange(n) / SR
        scoop = 1 - 0.07 * np.exp(-t / 0.035)
        out = np.zeros(n)
        for f, a in ((370.0, 1.0), (466.2, 0.8), (554.4, 0.7), (370.0 * 1.004, 0.6), (466.2 * 0.996, 0.5)):
            out += a * harmonics(f * scoop * (1 + 0.004 * np.sin(2 * np.pi * 6 * t)), slope=0.9, top=12_000.0)
        out = np.tanh(out * 0.9)
        out = sos_filter(out, "highpass", 180)
        return out * env(n, 0.012, min(0.06, d / 3))

    tr = stereo(1.6)
    place(tr, pan(blast(0.15), 0), 0.0)
    place(tr, pan(blast(0.15), 0), 0.24)
    place(tr, pan(blast(0.95), 0), 0.48)
    return finish(room(tr, wet=0.16, length=1.2), loud_db=-13.0)


def crickets():
    d = 3.4
    tr = stereo(d)
    for f, period, pulses, p, start in ((4400, 0.56, 3, -0.45, 0.05), (4950, 0.63, 4, 0.4, 0.27), (4150, 0.74, 3, 0.05, 0.16)):
        at = start
        while at < d - 0.3:
            for k in range(pulses):
                n = int(0.017 * SR)
                t = np.arange(n) / SR
                ff = f * (1 + 0.01 * RNG.standard_normal())
                tone = np.sin(2 * np.pi * ff * t) + 0.12 * np.sin(2 * np.pi * 2 * ff * t)
                shape = np.sin(np.pi * t / t[-1]) ** 2
                place(tr, pan(tone * shape * (0.8 + 0.2 * RNG.random()), p), at + k * 0.031)
            at += period * (1 + 0.04 * RNG.standard_normal())
    hush = sos_filter(RNG.standard_normal(int(d * SR)), "lowpass", 900) * 0.006
    tr += np.stack([hush, np.roll(hush, 997)])
    out = finish(room(tr, wet=0.25, length=1.0), loud_db=-20.0)
    return out


def clap(bright=1.0):
    n = int(0.12 * SR)
    t = np.arange(n) / SR
    centre = (1100 + 700 * RNG.random()) * bright
    burst = resonator(RNG.standard_normal(n), centre, centre * 1.1) * decay(n, 0.018 + 0.01 * RNG.random())
    # Hands meet in a few quick bursts.
    for _ in range(2):
        k = int(RNG.uniform(0.003, 0.009) * SR)
        burst[k:] += 0.5 * resonator(RNG.standard_normal(n - k), centre * 1.2, centre) * decay(n - k, 0.01)
    thump = np.sin(2 * np.pi * 190 * t) * decay(n, 0.012) * 0.3
    return burst + thump


def golf_clap():
    tr = stereo(3.0)
    for p, start, interval, count in ((-0.35, 0.0, 0.44, 6), (0.3, 0.13, 0.49, 5), (0.0, 0.27, 0.52, 4)):
        at = start
        for _ in range(count):
            place(tr, pan(clap(0.9) * (0.5 + 0.2 * RNG.random()), p), at)
            at += interval * (1 + 0.05 * RNG.standard_normal())
    return finish(room(tr, wet=0.22, length=0.8), loud_db=-17.0)


def applause():
    d = 3.6
    tr = stereo(d)
    for _ in range(36):
        p = RNG.uniform(-0.9, 0.9)
        at = RNG.uniform(0, 0.35)
        interval = RNG.uniform(0.11, 0.2)
        level = RNG.uniform(0.3, 1.0)
        while at < d - 0.2:
            swell = min(1.0, (at + 0.2) / 0.6) * (1 if at < d - 1.2 else max(0.0, (d - 0.2 - at) / 1.0))
            place(tr, pan(clap() * level * swell * RNG.uniform(0.6, 1.0), p), at)
            at += interval * (1 + 0.1 * RNG.standard_normal())
    return finish(room(tr, wet=0.3, length=1.4), loud_db=-14.0)


def brass(freq, d, open_time=0.08, bright=2200.0, vib=0.0, slide=0.03):
    """A brass note (trombone, trumpet): a buzz whose tone opens up as it's played."""
    n = int(d * SR)
    t = np.arange(n) / SR
    f = freq * (1 - slide * np.exp(-t / 0.04)) * (1 + vib * np.sin(2 * np.pi * 5.4 * t) * np.minimum(1, t / 0.3))
    buzz = harmonics(f, slope=1.0, top=12_000.0)
    # Opening the bell (or the mute): low-pass rising from dull to bright, block by block.
    out = np.zeros(n)
    block = 480
    zi = None
    for i in range(0, n, block):
        tt = t[i]
        cut = 350 + bright * (1 - np.exp(-tt / open_time))
        sos = signal.butter(2, min(cut, SR / 2 - 200), btype="lowpass", fs=SR, output="sos")
        if zi is None:
            zi = signal.sosfilt_zi(sos) * 0
        out[i : i + block], zi = signal.sosfilt(sos, buzz[i : i + block], zi=zi)
    breath = sos_filter(RNG.standard_normal(n), "bandpass", [800, 3000]) * 0.03
    return np.tanh(1.6 * (out + breath)) * env(n, 0.02, 0.07)


def sad_trombone():
    tr = stereo(3.2)
    notes = ((233.08, 0.42), (220.0, 0.42), (207.65, 0.42), (196.0, 1.6))
    at = 0.0
    for i, (f, d) in enumerate(notes):
        last = i == len(notes) - 1
        x = brass(f, d, open_time=0.12, bright=1500.0, vib=0.018 if last else 0.0, slide=0.04)
        if last:
            # The last one sags as it dies away.
            n = len(x)
            sag = np.exp(-np.arange(n) / SR / 1.2)
            x *= sag
        place(tr, pan(x, 0), at)
        at += d + 0.03
    return finish(room(tr, wet=0.15, length=1.0))


def snare(n_s=0.25, level=1.0):
    n = int(n_s * SR)
    t = np.arange(n) / SR
    tone = (np.sin(2 * np.pi * 185 * t) + 0.6 * np.sin(2 * np.pi * 330 * t)) * decay(n, 0.05)
    rattle = sos_filter(RNG.standard_normal(n), "bandpass", [1800, 8000]) * decay(n, 0.09)
    return (0.6 * tone + 1.2 * rattle) * level * env(n, 0, 0.02)


def tom(f0=170, f1=105, d=0.5):
    n = int(d * SR)
    t = np.arange(n) / SR
    f = f1 + (f0 - f1) * np.exp(-t / 0.08)
    body = np.sin(2 * np.pi * phase_of(f)) * decay(n, 0.22)
    stick = sos_filter(RNG.standard_normal(n), "bandpass", [2000, 6000]) * decay(n, 0.006) * 0.4
    return (body + stick) * env(n, 0, 0.05)


def kick(d=0.5):
    n = int(d * SR)
    t = np.arange(n) / SR
    f = 50 + 110 * np.exp(-t / 0.035)
    return np.sin(2 * np.pi * phase_of(f)) * decay(n, 0.18) * env(n, 0, 0.05)


def cymbal(d=None, tau=0.45):
    """Metal: six square waves at clashing frequencies (like a drum machine's), kept to the
    top of the range, plus hiss."""
    d = d or tau * 7
    n = int(d * SR)
    t = np.arange(n) / SR
    metal = np.zeros(n)
    for f in (205.3, 304.4, 369.6, 522.7, 540.0, 800.0):
        metal += np.sign(np.sin(2 * np.pi * f * 1.7 * t))
    metal = sos_filter(metal, "bandpass", [6500, 13000], order=3)
    hiss = sos_filter(RNG.standard_normal(n), "highpass", 5000) * 0.8
    shape = decay(n, tau) * (1 + 1.5 * decay(n, 0.01)) * env(n, 0, 0.1)
    return (0.5 * metal + hiss) * shape


def ba_dum_tss():
    tr = stereo(4.2)
    place(tr, pan(snare(0.4), -0.15), 0.0)
    place(tr, pan(tom(150, 95, 1.0) * 1.1 + kick(1.0) * 0.8, 0.1), 0.17)
    place(tr, pan(cymbal(tau=0.5) * 0.55, 0.3), 0.47)
    return finish(room(tr, wet=0.18, length=0.9))


def drumroll():
    tr = stereo(6.0)
    at, i = 0.0, 0
    while at < 1.7:
        level = 0.25 + 0.75 * (at / 1.7) ** 1.4
        hand = 0.85 if i % 2 else 1.0
        place(tr, pan(snare(0.12, level * hand * RNG.uniform(0.85, 1.0)), -0.1 if i % 2 else 0.1), at)
        at += 1 / 26 * (1 + 0.08 * RNG.standard_normal())
        i += 1
    place(tr, pan(cymbal(tau=0.5) * 0.8, 0.25), 1.75)
    place(tr, pan(kick(1.0) * 1.2, 0), 1.75)
    place(tr, pan(snare(0.4, 1.0), 0), 1.75)
    return finish(room(tr, wet=0.2, length=1.0))


def ta_da():
    tr = stereo(2.2)
    chord = (261.63, 329.63, 392.0, 523.25)
    for k, f in enumerate(chord):
        p = -0.3 + 0.2 * k
        place(tr, pan(brass(f, 0.13, open_time=0.02, bright=3500, slide=0.02) * 0.5, p), 0.0)
        place(tr, pan(brass(f * 1.0, 1.25, open_time=0.15, bright=3500, vib=0.008, slide=0.02) * 0.5, p), 0.2)
    return finish(room(tr, wet=0.2, length=1.2))


def ding():
    n = int(5.0 * SR)
    t = np.arange(n) / SR
    f = 2093.0
    out = np.zeros(n)
    for ratio, level, tau in ((1.0, 1.0, 0.75), (1.0018, 0.6, 0.7), (2.0, 0.25, 0.45), (2.76, 0.35, 0.5), (5.40, 0.15, 0.2), (8.93, 0.08, 0.09)):
        out += level * np.sin(2 * np.pi * f * ratio * t) * decay(n, tau)
    strike = sos_filter(RNG.standard_normal(n), "highpass", 3000) * decay(n, 0.004) * 0.6
    tr = pan((out + strike) * env(n, 0, 0.3), 0.1)
    return finish(room(tr, wet=0.2, length=1.2), loud_db=-15.0)


def boing():
    n = int(2.0 * SR)
    t = np.arange(n) / SR
    base = 140 + 260 * (1 - np.exp(-t / 0.03))
    wobble = 1 + 0.35 * np.exp(-t / 0.22) * np.sin(2 * np.pi * 10.5 * t)
    f = base * wobble
    ph = phase_of(f)
    x = np.sin(2 * np.pi * ph) + 0.35 * np.sin(4 * np.pi * ph) + 0.15 * np.sin(6 * np.pi * ph)
    x *= decay(n, 0.33) * np.minimum(1, t / 0.004) * env(n, 0, 0.1)
    return finish(room(pan(x, 0), wet=0.1, length=0.4))


def pew_pew():
    tr = stereo(1.0)
    for at, p in ((0.0, -0.4), (0.24, 0.4)):
        n = int(0.6 * SR)
        t = np.arange(n) / SR
        f = 260 + 2600 * np.exp(-t / 0.055)
        ph = phase_of(f)
        x = np.sin(2 * np.pi * ph) + 0.3 * np.sin(6 * np.pi * ph)
        x *= decay(n, 0.09) * np.minimum(1, t / 0.002) * env(n, 0, 0.05)
        place(tr, pan(x, p), at)
    return finish(room(tr, wet=0.15, length=0.5))


def campfire():
    d = 4.0
    n = int(d * SR)
    tr = stereo(d)
    rumble = sos_filter(np.cumsum(RNG.standard_normal(n)), "bandpass", [60, 450])
    rumble /= np.max(np.abs(rumble))
    flutter = 0.6 + 0.4 * sos_filter(RNG.standard_normal(n), "lowpass", 3)
    tr += np.stack([rumble, np.roll(rumble, 4801)]) * 0.08 * flutter
    at = 0.0
    while at < d - 0.1:
        clicks = RNG.integers(1, 5)
        centre = RNG.uniform(1800, 7000)
        level = RNG.lognormal(-1.2, 0.7)
        p = RNG.uniform(-0.7, 0.7)
        for _ in range(clicks):
            m = int(0.02 * SR)
            c = resonator(RNG.standard_normal(m), centre, centre * 0.8) * decay(m, 0.002 + 0.003 * RNG.random())
            place(tr, pan(c * level, p), at + RNG.uniform(0, 0.012))
        at += RNG.exponential(1 / 13)
    at = 0.3
    while at < d - 0.2:
        m = int(0.06 * SR)
        pop = resonator(RNG.standard_normal(m), RNG.uniform(700, 1400), 700) * decay(m, 0.012) * RNG.uniform(0.5, 1.2)
        place(tr, pan(pop, RNG.uniform(-0.5, 0.5)), at)
        at += RNG.exponential(1 / 1.6)
    tr *= env(n, 0.35, 0.5)
    return finish(room(tr, wet=0.12, length=0.6), loud_db=-18.0)


def spooky():
    n = int(3.2 * SR)
    t = np.arange(n) / SR
    # A theremin gliding through a minor tune: A4 E5 D5 C5 A4.
    pts = ((0.0, 440.0), (0.55, 659.3), (1.15, 587.3), (1.75, 523.3), (2.35, 440.0), (3.2, 415.3))
    f = np.interp(t, [p[0] for p in pts], [p[1] for p in pts])
    f = signal.savgol_filter(f, 4801, 2)
    f *= 1 + 0.018 * np.sin(2 * np.pi * 6.2 * t)
    ph = phase_of(f)
    x = np.sin(2 * np.pi * ph) + 0.12 * np.sin(4 * np.pi * ph)
    x *= env(n, 0.25, 0.6) * (0.85 + 0.15 * np.sin(2 * np.pi * 0.7 * t))
    return finish(room(pan(x, 0), wet=0.4, length=2.2), loud_db=-16.0)


SOUNDS = {
    "quack": quack,
    "airhorn": airhorn,
    "crickets": crickets,
    "golf-clap": golf_clap,
    "applause": applause,
    "sad-trombone": sad_trombone,
    "ba-dum-tss": ba_dum_tss,
    "drumroll": drumroll,
    "ta-da": ta_da,
    "ding": ding,
    "boing": boing,
    "pew-pew": pew_pew,
    "campfire": campfire,
    "spooky": spooky,
}


def write_wav(path, track):
    pcm = np.clip(track.T, -1, 1)
    data = (pcm * 32767).astype("<i2").tobytes()
    with wave.open(path, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(data)


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "sounds"
    os.makedirs(out, exist_ok=True)
    for name, make in SOUNDS.items():
        track = make()
        assert track.shape[0] == 2 and track.shape[1] / SR <= 10.0, name
        write_wav(os.path.join(out, f"{name}.wav"), track)
        print(f"{name}: {track.shape[1] / SR:.2f} s")


if __name__ == "__main__":
    main()
