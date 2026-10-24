"""Synthesise the launch-video soundtrack: a low pulse bed, ticks, whooshes,
impacts and a riser. No samples, no licensing. 34 s, 48 kHz stereo."""
import math
import random
import struct
import wave

SR = 48_000
DUR = 34.0
N = int(SR * DUR)
random.seed(7)

left = [0.0] * N
right = [0.0] * N


def add(t0, samples, pan=0.0, gain=1.0):
    """Add mono samples at t0. pan -1..1."""
    start = int(t0 * SR)
    gl = gain * math.sqrt((1 - pan) / 2)
    gr = gain * math.sqrt((1 + pan) / 2)
    for i, s in enumerate(samples):
        j = start + i
        if 0 <= j < N:
            left[j] += s * gl
            right[j] += s * gr


def env_exp(t, tau):
    return math.exp(-t / tau)


def kick(dur=0.38):
    out = []
    phase = 0.0
    for i in range(int(dur * SR)):
        t = i / SR
        f = 46 + 92 * math.exp(-t / 0.045)
        phase += 2 * math.pi * f / SR
        a = env_exp(t, 0.11) * (1 - math.exp(-t / 0.002))
        out.append(math.sin(phase) * a)
    return out


def sub(dur=1.1, f=55.0):
    return [math.sin(2 * math.pi * f * i / SR) * env_exp(i / SR, 0.42) for i in range(int(dur * SR))]


def tick(f=2400.0, dur=0.035):
    out = []
    for i in range(int(dur * SR)):
        t = i / SR
        e = env_exp(t, 0.006)
        out.append((math.sin(2 * math.pi * f * t) * 0.7 + random.uniform(-1, 1) * 0.3) * e)
    return out


def whoosh(dur=0.55, reverse=False):
    out = []
    prev = 0.0
    for i in range(int(dur * SR)):
        t = i / SR
        x = random.uniform(-1, 1)
        hp = x - prev * 0.92
        prev = x
        u = t / dur
        e = u ** 2 if not reverse else (1 - u) ** 2
        out.append(hp * e * 0.55)
    return out


def impact(dur=0.9):
    out = []
    for i in range(int(dur * SR)):
        t = i / SR
        a = env_exp(t, 0.16)
        n = random.uniform(-1, 1) * a * 0.25
        out.append(math.sin(2 * math.pi * (52 - 18 * min(t, 0.4)) * t) * a + n)
    return out


def riser(dur=2.4):
    out = []
    for i in range(int(dur * SR)):
        t = i / SR
        u = t / dur
        n = random.uniform(-1, 1) * (u ** 2.4) * 0.22
        s = math.sin(2 * math.pi * (180 + 900 * u * u) * t) * (u ** 3) * 0.10
        out.append(n + s)
    return out


def shimmer(dur=2.2):
    out = []
    for i in range(int(dur * SR)):
        t = i / SR
        a = env_exp(t, 0.55)
        v = sum(math.sin(2 * math.pi * f * t + p) for f, p in [(1046, 0), (1568, 1.3), (2093, 2.1)])
        out.append(v / 3 * a * 0.14)
    return out


# --- bed: a 60 bpm sub pulse, quiet, from 0 to 30 --------------------------
for beat in range(0, 31):
    add(beat, sub(0.9, 55.0), pan=0.0, gain=0.16 if beat % 2 == 0 else 0.10)
# a slow pad under the first half
for i in range(int(30 * SR)):
    t = i / SR
    trem = 0.65 + 0.35 * math.sin(2 * math.pi * 0.07 * t)
    s = (math.sin(2 * math.pi * 55 * t) + 0.5 * math.sin(2 * math.pi * 110 * t)) * 0.012 * trem
    j = i
    left[j] += s
    right[j] += s * 0.92

# --- hits ----------------------------------------------------------------
for t in [0.0, 8.0, 16.0, 30.0]:
    add(t, impact(), pan=0.0, gain=0.55)
for t in [4.0, 10.5, 21.0, 26.0]:
    add(t - 0.42, whoosh(0.6, reverse=True), pan=-0.25, gain=0.35)
add(16.0 - 0.42, whoosh(0.6, reverse=True), pan=0.25, gain=0.35)

# --- ticks: gates, cards, checks ----------------------------------------
for t in [0.55, 1.15, 1.75, 2.35]:
    add(t, tick(2100), pan=0.15, gain=0.18)
for t in [4.3, 4.55, 4.8]:
    add(t, tick(900, 0.05), pan=-0.2, gain=0.16)
for t in [11.4, 12.2, 13.0, 14.0]:
    add(t, tick(2600), pan=0.2, gain=0.16)
for t in [16.5, 17.3, 18.2, 19.2]:
    add(t, tick(1800), pan=-0.1, gain=0.15)
for t in [21.7, 22.5, 23.3]:
    add(t, tick(1200, 0.045), pan=0.1, gain=0.15)
for t in [27.0, 27.6, 28.2, 29.0]:
    add(t, tick(2300), pan=-0.2, gain=0.16)
for t in [30.7, 31.2]:
    add(t, tick(3000), pan=0.0, gain=0.14)

# --- transitions --------------------------------------------------------
add(6.55, whoosh(0.35), pan=0.2, gain=0.18)
add(7.85, whoosh(0.4, reverse=True), pan=-0.2, gain=0.22)
add(26.6, tick(300, 0.12), pan=0.0, gain=0.25)   # the break
add(27.5, riser(), pan=0.0, gain=0.22)
add(30.2, shimmer(3.4), pan=0.0, gain=0.30)

# --- tail: keep the last card alive to the final frame --------------------
for i in range(int(3.9 * SR)):
    t = i / SR
    if t < 0.6:
        env = t / 0.6
    elif t > 3.0:
        env = max(0.0, (3.9 - t) / 0.9)
    else:
        env = 1.0
    trem = 0.75 + 0.25 * math.sin(2 * math.pi * 0.6 * t)
    v = (math.sin(2 * math.pi * 55 * t) + 0.4 * math.sin(2 * math.pi * 110 * t)) * 0.018 * env * trem
    j = int((30.0 + t) * SR)
    if j < N:
        left[j] += v
        right[j] += v * 0.94
for t in [32.0, 33.0]:
    add(t, sub(0.8, 55.0), pan=0.0, gain=0.05)


def write_wav(path):
    peak = max(max(abs(x) for x in left), max(abs(x) for x in right))
    norm = 0.82 / peak if peak > 0 else 1.0
    with wave.open(path, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SR)
        frames = bytearray()
        for i in range(N):
            l = max(-1, min(1, left[i] * norm))
            r = max(-1, min(1, right[i] * norm))
            frames += struct.pack("<hh", int(l * 32767), int(r * 32767))
        w.writeframes(bytes(frames))
    print("wrote", path, "peak", round(peak, 3), "norm", round(norm, 3))


write_wav("media/bed.wav")
