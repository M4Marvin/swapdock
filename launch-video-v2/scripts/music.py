"""Upbeat, light soundtrack for the launch film. Pure Python, no samples.

120 BPM, F major, 30 s. Four-on-the-floor kick, offbeat hats, plucky arp,
warm pad and a soft final hit. Mastered quiet enough to sit under the film.
"""
import math
import random
import struct
import wave

SR = 48_000
DUR = 30.0
N = int(SR * DUR)
BPM = 120.0
BEAT = 60.0 / BPM
BAR = 4.0 * BEAT
random.seed(11)

left = [0.0] * N
right = [0.0] * N


def add(t0, samples, pan=0.0, gain=1.0):
    start = int(t0 * SR)
    gl = gain * math.sqrt((1.0 - pan) * 0.5)
    gr = gain * math.sqrt((1.0 + pan) * 0.5)
    for i, s in enumerate(samples):
        j = start + i
        if 0 <= j < N:
            left[j] += s * gl
            right[j] += s * gr


def fade(t, attack, release, dur):
    if t < attack:
        return t / attack
    if t > dur - release:
        return max(0.0, (dur - t) / release)
    return 1.0


def kick(dur=0.30):
    out = []
    phase = 0.0
    for i in range(int(dur * SR)):
        t = i / SR
        f = 48.0 + 82.0 * math.exp(-t / 0.045)
        phase += 2.0 * math.pi * f / SR
        a = math.exp(-t / 0.11) * (1.0 - math.exp(-t / 0.002))
        out.append(math.sin(phase) * a)
    return out


def noise(n, seed):
    rnd = random.Random(seed)
    return [rnd.uniform(-1.0, 1.0) for _ in range(n)]


def highpass(samples, cutoff):
    a = 1.0 / (1.0 + 2.0 * math.pi * cutoff / SR)
    out = []
    prev = 0.0
    for x in samples:
        prev = a * (prev + x - (out[-1] if out else 0.0))
        out.append(prev)
    return out


def hat(dur=0.055, seed=1):
    n = int(dur * SR)
    out = highpass(noise(n, seed), 6500.0)
    return [x * math.exp(-(i / SR) / 0.018) for i, x in enumerate(out)]


def clap(dur=0.18, seed=2):
    n = int(dur * SR)
    base = highpass(noise(n, seed), 1200.0)
    out = []
    for i, x in enumerate(base):
        t = i / SR
        burst = 0.0
        for k in range(3):
            d = t - k * 0.011
            if d >= 0.0:
                burst += math.exp(-d / 0.012)
        out.append(x * burst * math.exp(-t / 0.10) * 0.55)
    return out


def bass(freq, dur=0.22, amp=1.0):
    out = []
    phase = 0.0
    for i in range(int(dur * SR)):
        t = i / SR
        phase += 2.0 * math.pi * freq / SR
        v = math.sin(phase) + 0.28 * math.sin(2.0 * phase) + 0.10 * math.sin(3.0 * phase)
        out.append(v * math.exp(-t / 0.16) * (1.0 - math.exp(-t / 0.004)) * amp)
    return out


def pluck(freq, dur=0.32, amp=1.0):
    out = []
    phase = 0.0
    for i in range(int(dur * SR)):
        t = i / SR
        phase += 2.0 * math.pi * freq / SR
        v = math.sin(phase) + 0.38 * math.sin(2.0 * phase) + 0.14 * math.sin(3.0 * phase)
        out.append(v * math.exp(-t / 0.12) * (1.0 - math.exp(-t / 0.003)) * amp)
    return out


def pad(freqs, dur, amp=1.0):
    out = []
    for i in range(int(dur * SR)):
        t = i / SR
        v = 0.0
        for f in freqs:
            v += math.sin(2.0 * math.pi * f * t)
            v += 0.22 * math.sin(2.0 * math.pi * f * 2.0 * t)
            v += 0.07 * math.sin(2.0 * math.pi * f * 3.0 * t)
        env = fade(t, 0.18, 0.5, dur)
        duck = 1.0 - 0.30 * math.exp(-((t % BEAT) / 0.11))
        out.append(v / (len(freqs) * 1.3) * env * duck * amp)
    return out


def whoosh(dur=0.5, seed=5):
    n = int(dur * SR)
    raw = noise(n, seed)
    out = []
    lp = 0.0
    for i, x in enumerate(raw):
        t = i / SR
        a = 0.03 + 0.22 * (t / dur)
        lp += a * (x - lp)
        env = math.sin(math.pi * min(1.0, t / dur)) ** 2
        out.append(lp * env)
    return out


def crash(dur=1.6, seed=9):
    n = int(dur * SR)
    out = highpass(noise(n, seed), 3000.0)
    return [x * math.exp(-(i / SR) / 0.55) * 0.5 for i, x in enumerate(out)]


F = [174.61, 220.00, 261.63]
C = [196.00, 261.63, 329.63]
Dm = [146.83, 174.61, 220.00]
Bb = [174.61, 233.08, 293.66]
PROG = [F, C, Dm, Bb]
ROOTS = [87.31, 65.41, 73.42, 58.27]
ARPS = [
    [349.23, 440.00, 523.25, 698.46],
    [392.00, 523.25, 659.25, 783.99],
    [293.66, 349.23, 440.00, 587.33],
    [349.23, 466.16, 587.33, 698.46],
]

# --- intro pad and arp -----------------------------------------------------
for bar in range(15):
    t0 = bar * BAR
    idx = bar % 4
    chord = PROG[idx]
    # pad, two detuned layers panned wide
    p = pad(chord, BAR, amp=0.95)
    add(t0, p, pan=-0.45, gain=0.18)
    p = pad([f * 1.002 for f in chord], BAR, amp=0.95)
    add(t0, p, pan=0.45, gain=0.18)

    # bass eighths
    root = ROOTS[idx]
    if bar >= 1:
        for i in range(8):
            note = root * (2.0 if i in (2, 6) else 1.0)
            add(t0 + i * 0.25, bass(note, 0.24, 0.55), pan=0.0, gain=0.30)

    # plucked arp, eighths with ping-pong echo
    if bar != 12 or True:
        pattern = [0, 1, 2, 3, 2, 1, 0, 2]
        for i, k in enumerate(pattern):
            t = t0 + i * 0.25
            if bar == 12 and i > 4:
                continue
            f = ARPS[idx][k]
            pan = -0.35 if i % 2 == 0 else 0.35
            add(t, pluck(f, 0.30, 0.50), pan=pan, gain=0.16)
            add(t + 0.1875, pluck(f, 0.26, 0.35), pan=-pan, gain=0.055)

# --- drums -----------------------------------------------------------------
for bar in range(15):
    t0 = bar * BAR
    if bar >= 1 and bar != 12:
        for b in range(4):
            add(t0 + b * BEAT, kick(), pan=0.0, gain=0.45)
    if bar == 0:
        add(1.0, kick(), pan=0.0, gain=0.30)
        add(3.0, kick(), pan=0.0, gain=0.30)
    if bar >= 2 and bar != 12:
        for b in (1, 3):
            add(t0 + b * BEAT, clap(), pan=0.0, gain=0.20)
    if bar >= 1 and bar != 12:
        for i in range(8):
            t = t0 + i * 0.25 + 0.25
            if i == 7:
                continue
            add(t, hat(seed=bar * 10 + i), pan=0.25 if i % 2 else -0.25, gain=0.055)
    if bar == 12:
        # snare-ish build into the last bar
        for i in range(8):
            add(t0 + 1.0 + i * 0.125, clap(0.09, seed=100 + i), pan=0.0, gain=0.08 + i * 0.012)

# --- scene whooshes (soft, never louder than the music) --------------------
for cut in (4.0, 8.0, 12.0, 17.0, 21.0, 25.0):
    add(cut - 0.32, whoosh(0.55, seed=int(cut * 7)), pan=-0.15, gain=0.10)
    add(cut - 0.10, whoosh(0.35, seed=int(cut * 7) + 1), pan=0.2, gain=0.05)

# --- final hit and tail ----------------------------------------------------
final = [174.61, 220.00, 261.63, 349.23]
add(27.5, pad(final, 2.5, amp=1.0), pan=0.0, gain=0.22)
add(27.5, kick(0.5), pan=0.0, gain=0.55)
add(27.5, bass(87.31, 0.8, 0.7), pan=0.0, gain=0.25)
add(27.5, crash(), pan=0.0, gain=0.18)
add(29.0, crash(1.0, seed=12), pan=0.3, gain=0.06)

# --- normalise and write ---------------------------------------------------
peak = max(max(abs(x) for x in left), max(abs(x) for x in right))
scale = 0.80 / peak
with wave.open("media/bed.wav", "w") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(SR)
    frames = bytearray()
    for l, r in zip(left, right):
        frames += struct.pack("<hh", int(max(-1, min(1, l * scale)) * 32767),
                              int(max(-1, min(1, r * scale)) * 32767))
    w.writeframes(frames)
print("wrote media/bed.wav", "peak", round(peak, 3), "scale", round(scale, 3))
