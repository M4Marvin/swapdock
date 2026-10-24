# swapdock — launch film v2

A 30-second, benefit-first cut. [fframes](https://fframes.studio) draws every frame as SVG;
Skia renders it; ffmpeg encodes it. No footage, stock music or templates.

```text
out.mp4   1920x1080, 30 fps, 900 frames, h.264 + AAC, 30.0 s
```

## Why v2

v1 explained the machinery — journals, locks, crash recovery. v2 sells what a buyer feels:

| Time | Scene | Line |
|---|---|---|
| 0:00 | Ship | *SHIP AT 2PM. Nobody notices.* |
| 0:04 | Problem | *DOWNTIME IS A CHOICE.* A 502 flips to 200. |
| 0:08 | One command | `swapdock up blog --release 9c1f2ab` — build, start, health, flip, commit. |
| 0:12 | No window | Old and new beside each other; 200s keep moving; 15 s drain. |
| 0:17 | Rollback | `swapdock rollback blog`; v42 **bad deploy** → v41 **live**. |
| 0:21 | One box | No Kubernetes, no control plane; nginx + one binary. Stateful apps in one chip. |
| 0:25 | Outro | *Deploy on a Tuesday.* / *Make the next deploy boring.* |

The niche internals stay out. Simplicity, uptime, undo and cost are in.

## Sound

`scripts/music.py` synthesises a 30 s track: 120 BPM, F major, F–C–Dm–Bb,
four-on-the-floor kick, offbeat hats, plucked arpeggio, warm pad, soft whooshes on the cuts,
and a final hit. It is mastered deliberately quiet:

```text
-17.6 LUFS integrated   -1.5 dBFS true peak
```

```sh
python3 scripts/music.py
ffmpeg -i media/bed.wav -af "loudnorm=I=-18:TP=-1.5:LRA=5" -ar 48000 media/bed_master.wav
```

## Build and render

Prerequisites: Rust, libclang (bindgen for ffmpeg-sys), a Vulkan driver (lavapipe works headless),
and the Skia link libraries (`freetype`, `fontconfig`, `x264`).

```sh
cargo build --release
./target/release/launch-video timeline
./target/release/launch-video inspect
./target/release/launch-video strip Ship -n 8 --columns 4 --width 480
./target/release/launch-video frame 2.5s -o review
./target/release/launch-video render -o out.mp4
```

This render used the same extracted toolchain as v1:

```sh
export CARGO_HOME=/tmp/cargo-home
export CARGO_TARGET_DIR=/tmp/fframes-target
export FFMPEG_BINARIES_CACHE=/tmp/ffmpeg-cache
export LIBCLANG_PATH=/tmp/clang/usr/lib/llvm-18/lib
export BINDGEN_EXTRA_CLANG_ARGS="-isystem /usr/lib/gcc/x86_64-linux-gnu/13/include -isystem /usr/include -isystem /usr/include/x86_64-linux-gnu -isystem /tmp/clang/usr/lib/llvm-18/lib/clang/18/include"
export LIBRARY_PATH=/tmp/libs/usr/lib/x86_64-linux-gnu
export LD_LIBRARY_PATH=/tmp/libs/usr/lib/x86_64-linux-gnu
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
```

## Edition notes

- **Geist** for type, **IBM Plex Mono** for commands. IBM Plex is used because Geist Mono's
  ligatures turn `--release` into an em dash.
- Ports, releases and app names are illustrative UI states, not a screen capture.
- No voice-over. The on-screen text carries the argument; the score marks the cuts.
- v1 (34 s, technical) is kept in `../launch-video` for comparison.

## Files

```text
src/lib.rs        seven scenes, the video, background
src/main.rs       CLI, Skia Vulkan renderer, H.264 settings
scripts/music.py  generated soundtrack
media/            Geist + IBM Plex Mono, bed.wav
review/           contact sheets and key frames (git-ignored)
```
