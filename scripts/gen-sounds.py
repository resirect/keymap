#!/usr/bin/env python3
"""Generate Keymap's opening sounds (original, synthesized; no samples used).

Writes 48 kHz mono 16-bit WAV files to crates/keymap-app/assets/sounds/:

  ripple-first.wav  ~1.47 s, follows the first opening's ripple
  ripple-later.wav  ~0.73 s, later openings (same sound, shorter envelopes;
                    not sped up, so the pitch and character stay the same)
  open-quick.wav    ~0.18 s, used when motion is off
  select.wav        ~45 ms, a soft tick when a key is selected
  deselect.wav      ~45 ms, the same tick a little lower, for deselecting
  saved.wav         ~150 ms, a gentle tone after a binding is saved

The sound is a soft airy swell (filtered noise) with a few very quiet sine
partials that enter in step with the keys spreading outward and fade with
them; no separate finishing note. Levels are about 12 dB below the first
version (RMS ~-38 dBFS at full volume). Deterministic: same output every run.

Usage: python3 scripts/gen-sounds.py
"""

import math
import os
import random
import struct
import wave

RATE = 48000
OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "keymap-app", "assets", "sounds")
# Ripple durations, matching crates/keymap-app/src/motion.rs.
FIRST = 2.2 / 1.5
LATER = FIRST / 2
# A major-pentatonic cluster, kept below ~800 Hz so nothing is bright.
PARTIALS = [392.00, 523.25, 659.25, 783.99]
# Target loudness (RMS, linear) of the ripple; the quick cue matches it.
RMS = 10 ** (-38 / 20)


def smooth(x):
    """0..1 -> 0..1 with zero slope at both ends."""
    x = min(max(x, 0.0), 1.0)
    return math.sin(x * math.pi / 2) ** 2


def air(n, rng):
    """Band-limited noise: one-pole high-pass ~700 Hz, then two one-pole
    low-passes ~1.6 kHz, so it reads as breath rather than hiss."""
    hp_a = math.exp(-2 * math.pi * 700 / RATE)
    lp_a = math.exp(-2 * math.pi * 1600 / RATE)
    out, prev_in, hp, lp1, lp2 = [], 0.0, 0.0, 0.0, 0.0
    for _ in range(n):
        w = rng.uniform(-1, 1)
        hp = hp_a * (hp + w - prev_in)
        prev_in = w
        lp1 = lp_a * lp1 + (1 - lp_a) * hp
        lp2 = lp_a * lp2 + (1 - lp_a) * lp1
        out.append(lp2)
    peak = max(abs(v) for v in out) or 1.0
    return [v / peak for v in out]


def ripple(total):
    n = int(round(total * RATE))
    rng = random.Random(7)
    noise = air(n, rng)
    stagger, key = total * 4 / 11, total * 7 / 11
    out = []
    for i in range(n):
        t = i / RATE
        x = t / total
        # Breath swells while keys spread, then thins out as they settle.
        a = smooth(x / 0.5) * (1 - smooth((x - 0.4) / 0.6))
        v = 0.30 * a * noise[i]
        # Partials enter centre-out, like the keys (lowest first).
        for k, f in enumerate(PARTIALS):
            on = stagger * k / (len(PARTIALS) - 1)
            if t < on:
                continue
            u = t - on
            # Slow swell in, long decay, and gone before the clip ends.
            env = smooth(u / (key * 0.5)) * math.exp(-2.6 * u / key) * (1 - smooth((x - 0.7) / 0.3))
            shimmer = 1 + 0.08 * math.sin(2 * math.pi * (5.0 + k * 0.6) * t)
            amp = 0.07 / (1 + 0.7 * k)
            # Two slightly detuned copies give a soft chorus.
            v += amp * env * shimmer * 0.5 * (math.sin(2 * math.pi * f * t) + math.sin(2 * math.pi * f * 1.003 * t + k))
        out.append(v)
    return finish(out)


def quick():
    total = 0.18
    n = int(round(total * RATE))
    out = []
    for i in range(n):
        t = i / RATE
        env = smooth(t / 0.04) * math.exp(-t / 0.06)
        v = sum(0.2 / (1 + k) * math.sin(2 * math.pi * f * t) for k, f in enumerate(PARTIALS[:3]))
        out.append(env * v)
    return finish(out)


def tick(freq):
    """A rounded sine tick: 4 ms raised-cosine attack, quick decay, faded
    to silence by the end, with a faint lower octave for body."""
    total = 0.045
    n = int(round(total * RATE))
    out = []
    for i in range(n):
        t = i / RATE
        env = smooth(t / 0.004) * math.exp(-t / 0.012) * (1 - smooth((t - 0.025) / 0.02))
        out.append(env * (math.sin(2 * math.pi * freq * t) + 0.3 * math.sin(2 * math.pi * freq / 2 * t)))
    return effect(out, TICK_DB)


def saved():
    """One soft open fifth (D5 + A5), no melody: slow attack, gentle decay."""
    total = 0.15
    n = int(round(total * RATE))
    out = []
    for i in range(n):
        t = i / RATE
        env = smooth(t / 0.015) * math.exp(-t / 0.05) * (1 - smooth((t - 0.09) / 0.06))
        out.append(env * (math.sin(2 * math.pi * 587.33 * t) + 0.5 * math.sin(2 * math.pi * 880.0 * t)))
    return effect(out, SAVED_DB)


def window_db(samples):
    """Loudest 400 ms energy (the loudness meters' momentary window), in dB."""
    w = int(0.4 * RATE)
    sq = [v * v for v in samples] + [0.0] * w
    acc = sum(sq[:w])
    best = acc
    for i in range(w, len(sq)):
        acc += sq[i] - sq[i - w]
        best = max(best, acc)
    return 10 * math.log10(best / w + 1e-30)


def effect(samples, rel_db):
    """Scale an interaction sound to `rel_db` relative to the opening sound,
    comparing energy over 400 ms windows: short sounds are judged by the
    energy they put into that window, not by their peak."""
    g = 10 ** ((OPENING_DB + rel_db - window_db(samples)) / 20)
    return [v * g for v in samples]


def finish(samples):
    """Soft fades at both ends (no clicks or abrupt endings), then scale to
    the target RMS over the audible part."""
    n = len(samples)
    fade_in, fade_out = int(0.01 * RATE), int(0.08 * RATE)
    for i in range(min(fade_in, n)):
        samples[i] *= smooth(i / fade_in)
    for i in range(min(fade_out, n)):
        samples[n - 1 - i] *= smooth(i / fade_out)
    loud = [v for v in samples if abs(v) > 1e-4] or [1.0]
    rms = math.sqrt(sum(v * v for v in loud) / len(loud)) or 1.0
    return [v * RMS / rms for v in samples]


def write(name, samples):
    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, name)
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(round(max(-1, min(1, v)) * 32767))) for v in samples))
    print(f"{path}: {len(samples) / RATE:.3f} s")


# Interaction sounds sit below the opening sound (in dB, 400 ms energy).
TICK_DB = -10
SAVED_DB = -6

if __name__ == "__main__":
    first = ripple(FIRST)
    OPENING_DB = window_db(first)
    write("ripple-first.wav", first)
    write("ripple-later.wav", ripple(LATER))
    write("open-quick.wav", quick())
    write("select.wav", tick(880.0))
    write("deselect.wav", tick(740.0))
    write("saved.wav", saved())
