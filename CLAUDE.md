# CLAUDE.md

Guidance for working on VRC Openface: webcam face tracking for VRChat in Rust
(MediaPipe models on ONNX Runtime → Unified Expressions → OSC).

## Commands

```sh
cargo build --release                       # app: target/release/vrc-openface.exe
cargo build --release --features directml   # with DirectML GPU inference
cargo test                                  # unit tests (mapping, OSC binding, OSCQuery parsing)
cargo clippy --all-targets                  # keep at zero warnings
cargo fmt                                   # rustfmt.toml: max_width 120
cargo run --release --example still_image -- face.jpg     # pipeline on a still image, JSON out
cargo run --release --example camera_check -- 0 640 480 30  # camera mode + measured FPS
uv run tools/convert_models.py              # regenerate models/*.onnx from Google's .task
```

`VRC_OPENFACE_CONFIG=<path>` points the app at a different settings file. Use it for
tests so the user's real settings are left alone.

## Architecture

The crate is a library (`src/lib.rs`) plus a thin binary (`src/main.rs`). Three threads:

- **UI** (`app.rs`, eframe/egui 0.37 with the `App::ui` API): draws from a cloned
  `Snapshot` and writes `Settings` back into `Shared`.
- **Tracker** (`tracker.rs`): camera → `FaceLandmarker` → optional tongue →
  `Processor` → `unified::evaluate_all` → `OscSender`. It publishes a `Snapshot` and
  asks egui to repaint. UI actions reach it as `Command`s over a crossbeam channel.
- **OSCQuery** (`oscquery.rs`): browses mDNS for `_oscjson._tcp` instances named
  `VRChat-Client-*` and polls `GET /avatar` every 2 s. `generation` is bumped on change
  so the tracker rebinds parameters only when needed.

### Face pipeline (`src/face/`)

This mirrors MediaPipe's face landmarker graph. When editing, keep these details:

- `crop_tensor` uses OpenCV `WARP_INVERSE_MAP` semantics: the affine maps destination
  pixels to source pixels, and pixel centres are on integers.
- Detector: 128×128 NHWC input in [-1, 1] with a zero border; 896 anchors
  (16×16×2 + 8×8×6); sigmoid scores ≥ 0.5; weighted merge of boxes with IoU > 0.3
  against the best box. ROI = 1.5× the box, rotated by the eye keypoints (0 → 1).
- Landmarks: 256×256 input in [0, 1] with a replicated border. Output 0 is 478×3
  points, output 1 is a presence **logit** (< 0 means tracking is lost). The next ROI
  comes from the landmarks: bounding box ×1.5, rotated along landmarks 33 → 263, so the
  detector runs only after tracking is lost.
- `LandmarkFilter` is a One-Euro filter whose velocity is normalised by face size
  (min cutoff 0.05, beta 80, derivative cutoff 1).
- The blendshape input is 146 filtered landmarks (`BLENDSHAPE_POINTS`) as **pixel** xy.
  Output is 52 scores, clamped to [0, 1], in `channels::MEDIAPIPE` order (index 0 is
  `_neutral`).
- ONNX Runtime allows one environment per process; `face::environment()` shares it.

Validate pipeline changes against the official MediaPipe Python package on the same
image. The mean absolute blendshape difference was about 0.01 when this was written.

### Channels, processing, mapping

- `channels.rs`: 52 MediaPipe channels followed by 6 tongue channels (`COUNT` = 58),
  the left/right `SYMMETRY` pairs (horizontal gaze pairs one eye's In with the other
  eye's Out), and the UI row grouping.
- `processing.rs`: neutral rescale (skips eye-look and tongue channels), gain,
  symmetry, One-Euro smoothing, then gaze. Neutral and gaze captures average 45 frames.
- `unified.rs`: `PARAMS` is the single source of truth for output parameters. Each
  entry is a `Formula` over channel names, and `dependencies()` derives the green
  "used" dots in the UI. To add a parameter, add one table entry. The tests check that
  names are unique and that every dependency resolves.
- `osc.rs`: `bind()` matches avatar parameters by their name after `v2/` (any prefix,
  e.g. `FT/v2/`). Float or int parameters become a `Binding::Float`; bools
  `Name1, Name2, Name4…` (+ `NameNegative`) become a `Binding::Binary`. Values are sent
  when they change, and everything is re-sent once per second.

### Camera (`camera.rs`)

On Windows, capture uses Media Foundation through the `windows` crate. It ranks
native modes by fps ≥ requested, then closest area, then MJPG > NV12 > YUY2, and asks
the source reader (advanced video processing) for RGB32. If the camera only offers
large modes, the processor scales to the requested width at the native aspect ratio.
nokhwa's MSMF backend misreported modes and delivered about 12 FPS on a Brio 500, which
is why it is only used on non-Windows targets.

## Conventions

- Match the surrounding style: small modules, `anyhow::Result`, `log` macros, no
  `unwrap` on fallible runtime paths, comments that explain *why*.
- Keep `cargo clippy --all-targets` warning-free and run `cargo fmt` before committing.
- Never commit `models/tongue_model.onnx` (git-ignored) or other third-party weights.
  `models/*.onnx` must stay reproducible with `tools/convert_models.py`.
- Commits use the generic identity configured in this repo's local git config.
