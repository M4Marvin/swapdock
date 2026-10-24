# swapdock — launch film

A 34-second launch film for swapdock. Everything is drawn in code:
[fframes](https://fframes.studio) builds an SVG tree for every frame, Skia renders it, ffmpeg encodes it.
No footage, no templates, no stock music.

```text
out.mp4   1920x1080, 30 fps, 1020 frames, h.264 + AAC, 34.0 s
```

## The film

| Time | Scene | Beat |
|---|---|---|
| 0:00 | Hook | A cutover happens in front of you. *"You didn't notice."* |
| 0:04 | Problem | Registry, nginx and the container disagree: v41, v42, v43. |
| 0:08 | Turn | Three chips collapse into one source of truth. |
| 0:10 | Green | Start beside the old; container health and `GET /health` gate the flip. |
| 0:16 | Flip | Traffic moves once; a 15 s drain ring; the registry commits last. |
| 0:21 | Stateful | A second writer is blocked by `writes_state = true`; replace is the path. |
| 0:26 | Recovery | The process dies mid-flip; the journal and evidence checks decide. |
| 0:30 | Outro | Logo, command, `Make the next deploy boring.` |

## Build and render

Prerequisites: Rust 1.99+, libclang (bindgen for ffmpeg-sys), a Vulkan driver (lavapipe works
headless), and the usual Skia link libraries (`freetype`, `fontconfig`, `x264`). On a normal
desktop these are system packages; on a bare server they can be extracted to `/tmp` and passed
through `LIBRARY_PATH` / `LD_LIBRARY_PATH`.

```sh
cargo build --release
./target/release/launch-video timeline
./target/release/launch-video inspect          # problems + font/text checks
./target/release/launch-video strip Hook -n 8  # contact sheet: strip.png
./target/release/launch-video frame 3.5s -o review
./target/release/launch-video render -o out.mp4
```

The build embeds every file in `media/`, so adding a font or changing `bed.wav` requires a rebuild.

### Environment used for this render

```sh
export CARGO_HOME=/tmp/cargo-home
export CARGO_TARGET_DIR=/tmp/fframes-target
export FFMPEG_BINARIES_CACHE=/tmp/ffmpeg-cache   # prebuilt FFmpeg, not ~/.cache
export LIBCLANG_PATH=/tmp/clang/usr/lib/llvm-18/lib
export BINDGEN_EXTRA_CLANG_ARGS="-isystem /usr/lib/gcc/x86_64-linux-gnu/13/include -isystem /usr/include -isystem /usr/include/x86_64-linux-gnu -isystem /tmp/clang/usr/lib/llvm-18/lib/clang/18/include"
export LIBRARY_PATH=/tmp/libs/usr/lib/x86_64-linux-gnu
export LD_LIBRARY_PATH=/tmp/libs/usr/lib/x86_64-linux-gnu
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
```

## Sound

`scripts/sound.py` synthesises a 34 s stereo bed: a 60 bpm sub pulse, ticks on the gates and checks,
reverse-noise whooshes on the cuts, impacts on the scene changes, a riser into the outro, and a tail.
It is mastered to about -16.3 LUFS with a -1.0 dBFS sample peak.

```sh
python3 scripts/sound.py
ffmpeg -i media/bed.wav -af "loudnorm=I=-15.5:TP=-1.5:LRA=9" ...
```

If you change scene durations, update the cue times in the script *and* `Duration::Seconds` in
`src/lib.rs`; the video duration is the sum of the scene durations.

## Edition notes

- Fonts: **Geist** (Sans) for type, **IBM Plex Mono** for commands. Both are OFL and live in `media/`.
  IBM Plex Mono is used because Geist Mono ligatures render `--` as an em dash, which makes commands lie.
- The two ports, release ids and run numbers are illustrative UI states, not a capture.
- The film deliberately has no voice-over. On-screen text carries the argument; the sound bed marks cuts.

## Where things are

```text
src/lib.rs        every scene, the video, and the chrome
src/main.rs       CLI, Skia Vulkan renderer, H.264 encoder settings
scripts/sound.py  generated soundtrack
media/            Geist + IBM Plex Mono, bed.wav
review/           contact sheets and key frames (git-ignored)
```
