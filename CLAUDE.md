# CLAUDE.md

Guidance for working on VRC Openface: webcam face tracking for VRChat in Rust
(MediaPipe models on ONNX Runtime → calibration → Unified Expressions → OSC).

## Commands

```sh
cargo build --release                         # DirectML on by default; --no-default-features for CPU only
cargo test                                    # mapping, calibration, OSC binding, parsers, interpolation
cargo clippy --all-targets                    # keep at zero warnings
cargo fmt                                     # rustfmt.toml: max_width 120
cargo run --release --example still_image -- face.jpg       # pipeline on a still image, JSON out
cargo run --release --example camera_check -- 0 640 480 30  # camera mode + measured FPS
uv run tools/convert_models.py                # regenerate models/*.onnx from Google's .task
uv run tools/make_icons.py                    # regenerate assets/icon.png and icon.ico
uv run tools/make_link_icons.py               # regenerate assets/github.png and x.png (top bar links)
```

`VRC_OPENFACE_DATA=<folder>` runs the app with its own data folder (settings,
calibrations, imported models). Use it for manual tests so the user's real data is
left alone. Note that a running VRChat will receive OSC from any test run.

## Architecture

Library crate (`src/lib.rs`) plus a thin binary (`src/main.rs`). Threads:

- **UI** (`app.rs`, eframe/egui 0.37, `App::ui` API): draws from a cloned `Snapshot`,
  edits `Settings` and the per-avatar `CalibrationStore` in `Shared`, and saves both
  every 2 s when they change. It sends `Command`s (capture neutral, undo, gaze, reload
  models) to the tracker. When idle it stops drawing (`ui_awake`).
- **Tracker** (`tracker.rs`): camera → `FaceLandmarker` → optional `TongueModel` →
  `Processor` → `Mapper` → pushes a `Frame` into the output feed and updates `Snapshot`.
  It also merges avatar information (see below) and records stage timings in `perf.rs`.
- **Output** (`output.rs`): sends OSC at 100 Hz. With interpolation on, it renders one
  frame interval in the past and blends the two latest frames; FT off sends
  `Mapper::neutral()`. `system::high_resolution_timer()` makes the sleeps accurate.
- **OSCQuery** (`oscquery.rs`): mDNS `_oscjson._tcp` → `GET /avatar` every 2 s.
- **Receiver** (`avatar.rs`): `/avatar/change` on the receive port (9001), plus
  `load_config` for VRChat's `LocalLow/VRChat/VRChat/OSC/usr_*/Avatars/<id>.json`
  (avatar name and parameters when OSCQuery is unavailable).

### Face pipeline (`src/face/`)

Mirrors MediaPipe's face landmarker graph:

- `crop_tensor` uses OpenCV `WARP_INVERSE_MAP` semantics (destination → source, pixel
  centres on integers).
- Detector: 128×128 NHWC in [-1, 1], zero border, 896 anchors (16×16×2 + 8×8×6), sigmoid
  ≥ 0.5, weighted merge at IoU > 0.3. ROI = box ×1.5, rotated by the eye keypoints.
- Landmarks: 256×256 in [0, 1], replicated border. Output 1 is a presence **logit**
  (< 0 = lost). The next ROI comes from the landmarks (×1.5, rotated along 33 → 263).
- `LandmarkFilter`: One-Euro on the landmarks, with velocity normalised by face size.
- Blendshapes: 146 filtered landmarks (`BLENDSHAPE_POINTS`) as **pixel** xy → 52 scores.
- `frown_metric`: roll-corrected mouth-corner height vs the inner-lip centre line,
  relative to mouth width; `processing.rs` maps -0.03…-0.15 to 0…1.
- Tongue: a square mouth crop turned with the mouth corners and reaching below the lower
  lip (`Mouth::find`), sampled like the face crops; the model's input sets the crop size and
  planes (luma or RGB), and its six outputs are the tongue channels.
- ONNX Runtime allows one environment per process; `face::environment()` shares it.

Validate pipeline changes against the official MediaPipe Python package on the same
image (mean blendshape difference ≈ 0.01 when this was written).

### Processing and mapping

- `channels.rs`: 52 MediaPipe channels + 6 tongue channels (`COUNT` = 58), symmetry
  groups (jaw and tongue are excluded from the global switch), display rows.
- `processing.rs`, in order: mesh frown source → One-Euro smoothing (strength 0..1,
  global or per channel) → per-avatar `ChannelCalibration` (min/max remap or override;
  L/R linked unless listed in `unlinked`) → symmetry → gaze minus `gaze_center`.
  Neutral capture averages 45 frames, sets each channel's `min` and keeps an undo copy.
- `mapping.rs`: `OUTPUTS` is the single source of truth (VRCFT name, friendly name,
  input channels, formula, signed, cat ears). The order of `inputs` is the order the
  formula receives values, and dependencies for the UI come from the same list.
- `osc.rs`: `bind()` matches parameters by their name after `v2/` (any prefix). A
  `Binding` can have a float address and binary bits at the same time; both are sent.
  Sends are deduplicated, bundled (≤ 4 KB) and fully repeated once per second.

### Camera (`camera.rs`)

Windows uses Media Foundation directly: it ranks native modes (fps ≥ requested, then
closest area, then MJPG > NV12 > YUY2) and asks the source reader for RGB32, scaled to
the requested width when the camera only offers large modes. nokhwa is used only on
other platforms; its MSMF backend misreported modes and delivered about 12 FPS on a Brio 500.

### Interface details (`app.rs`)

- Panel headers go through `header()`: title on the left (truncated), controls on the
  right, added right-to-left. Never let a right-to-left row overflow: it pushes the whole
  panel off the left edge. `header()` wraps the controls instead.
- Panel style: dashed outlines (`dashed_rect`), table rows with
  separators and no spacing between them, MediaPipe groups in dashed boxes.
- Panel links: each frame records row rects in `Rows` and the hovered row in
  `hover_now`; highlights use the previous frame's `hover`, and `draw_links` paints the
  lines after the three panels. When a hovered channel's expressions are not all in
  view, `pull_related` moves them into one block in the list next to the channel. The
  decision is kept while that channel stays hovered (`pulled`), otherwise it flickers.
- The MediaPipe panel is drawn first, so `hover_now` already holds a hovered channel
  when the Unified Expressions list is laid out.
- Use `min().max()` rather than `f32::clamp` with bounds taken from layout rects; empty
  rects (min > max) make `clamp` panic.
- The window icon is `assets/icon.png`; `build.rs` embeds `assets/icon.ico` in the exe
  with `winresource`.

### Other modules

`config.rs` (Settings, data dir, JSON persistence via write-then-rename), `i18n.rs`
(EN/JA/KO text table; CJK glyphs come from Windows fonts loaded in `app.rs`),
`theme.rs` (color schemes), `updater.rs` (GitHub releases of this repo; install renames
files in use to `*.old`, writes new ones and restarts), `system.rs` (DXGI adapters =
DirectML device ids, private memory, timer resolution).

## Conventions

- Match the surrounding style: small modules, `anyhow::Result`, `log` macros, no
  `unwrap` on fallible runtime paths, comments that explain *why*.
- Every user-visible string goes through `i18n::tr` with all three languages.
- Keep `cargo clippy --all-targets` warning-free and run `cargo fmt` before committing.
- Never commit `tongue_model.onnx` (git-ignored) or other third-party weights.
  `models/*.onnx` must stay reproducible with `tools/convert_models.py`.
- Commits use the generic identity in this repo's local git config.
- Versions: patch (0.3.0 → 0.3.1) for fixes, minor (→ 0.4.0) for features or
  behaviour changes, major (1.0.0) only for a big milestone or a breaking change to
  settings, calibrations or avatar setups. Record every change under `[Unreleased]` in
  `CHANGELOG.md`; on release, rename that section to the version and date, bump
  `Cargo.toml`, tag `vX.Y.Z` and use the section as the release notes.
- Release zips contain `vrc-openface.exe`, `DirectML.dll`, `models/` (without any
  tongue model), `README.md` and `LICENSE`; the updater requires the exe in the zip.
