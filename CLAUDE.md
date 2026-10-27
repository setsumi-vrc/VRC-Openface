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
cargo run --release --example tongue_check      # camera: tongue values and the mouth crop the model sees
cargo run --release --example tongue_device     # tongue model, CPU against DirectML on a synthetic input
uv run tools/make_link_icons.py               # regenerate assets/github.png and x.png (top bar links)
uv run tools/make_flags.py                    # regenerate assets/flags/*.png (language menu)
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
- Profiles (`CalibrationStore::profiles`, `Profile`): a named copy of one avatar's `AvatarCalibration` plus the `ProcessingSettings`; loading replaces both for the current avatar.
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
- Layout: top bar (status pill from `status_line`, Pause / Resume, Simple / Advanced switch
  (`Settings::advanced`), theme, settings, language; title, Pause label and links hidden below 940 pt). Simple (`simple_view`): the
  camera card as tall as the window allows next to the calibrate card (with profiles, `profiles_block`) and, under it, the MediaPipe panel (`mediapipe_panel(.., simple = true)`: own search, every parameter). Advanced: a strip across the top (`camera_strip`: calibration with profiles on the left, then the camera centered in the space left: controls up to `CAMERA_CONTROLS_MAX` wide next to a preview of the frame's shape), `tools_row` (search, switches and processing options for all tables, so panel headers carry no controls; its two
  groups stack below 1140 pt)
  and the three panels. Below 760 pt of window width (large interface scales) the cards stack, in a
  collapsible block above the panels in Advanced. The settings are an `egui::Modal` split into sections
  (`section`); its scroll area needs `min_scrolled_height` or the modal shrinks to half the window.
- The window is undecorated (`with_decorations(false)`): `top_bar` drags it (`StartDrag`), maximizes on
  double click and draws minimize / maximize / close (`window_button`). With no frame, `window_edges` turns
  the outer `RESIZE_BAND` points into resize handles (`BeginResize`) and raises a restored size below
  `MIN_WINDOW` once; the drag in `top_bar` ignores presses in that band. Close goes through the
  normal close request, so close-to-tray still applies.
- Controls come from `widgets.rs` (`button`, `icon_button`, `chip`, `pill`, `card`); icons are
  drawn by `icons.rs` on a 24 x 24 grid. Do not put glyph characters (arrows, check marks, emoji)
  in UI text: Segoe UI lacks them and they show as boxes. Add a `Glyph` instead.
- Fonts: `load_fonts` puts Segoe UI first (and its semibold as the `semibold` family) for
  non-CJK languages; Japanese, Korean and Chinese put their own CJK font first. A language's own
  name uses `native_font` so its text stays in one font.
- Panel style: rounded cards (`panel_frame`), 30 px rows with a 3 px value bar under the name
  (`paint_meter`) and no separators, MediaPipe groups set apart by a gap.
- Panel links: each frame records row rects in `Rows` and the hovered row in
  `hover_now`; highlights use the previous frame's `hover`, and `draw_links` paints the
  lines after the three panels. Rows are never moved: when the hovered row has been
  hovered for `REVEAL_AFTER`, `reveal` scrolls the first linked row that is out of view in
  another panel to the centre, once per hovered row (`revealed`, so it does not oscillate
  between several linked rows), and `draw_links` marks rows still out of view with an arrow.
- The MediaPipe panel is drawn first, so `hover_now` already holds a hovered channel
  when the Unified Expressions list is laid out.
- Use `min().max()` rather than `f32::clamp` with bounds taken from layout rects; empty
  rects (min > max) make `clamp` panic.
- `tray.rs` (Windows only, `tray-icon`): closing the window is cancelled and the window hidden
  with Win32 `ShowWindow`; the tray handlers show it again the same way, because a hidden window
  may not run the egui loop. Quit sets `quitting` so the close goes through. The right-click menu is a
  deferred egui viewport (title "VRC Openface menu", declared by `declare_menu` in every `ui` pass or egui
  drops it); the tray click handler positions and shows it with Win32 (`open_at`), and it hides itself on
  focus loss or Escape. Its Pause row toggles `Settings::ft_output` and saves it (the UI may not be running).
  eframe runs no `ui` while the main window is hidden, only `App::logic`, which is where `Tray::refresh`
  updates the icon's status dot and tooltip. `Settings::close_to_tray` decides whether close is cancelled.
- The window icon is `assets/icon.png`; `build.rs` embeds `assets/icon.ico` in the exe
  with `winresource`.

### Other modules

`config.rs` (Settings, data dir, JSON persistence via write-then-rename), `i18n/`
(`mod.rs` holds EN/JA/KO, one file per other language; CJK glyphs come from Windows fonts loaded in
`app.rs`, the selected language's font first),
`theme.rs` (color schemes), `updater.rs` (GitHub releases of this repo; install renames
files in use to `*.old`, writes new ones and restarts), `system.rs` (DXGI adapters =
DirectML device ids, private memory, timer resolution).

## Conventions

- Match the surrounding style: small modules, `anyhow::Result`, `log` macros, no
  `unwrap` on fallible runtime paths, comments that explain *why*.
- Every user-visible string goes through `i18n::tr` with every language (the completeness test fails otherwise).
- Keep `cargo clippy --all-targets` warning-free and run `cargo fmt` before committing.
- Never commit `tongue_model.onnx` (git-ignored) or other third-party weights.
  `models/*.onnx` must stay reproducible with `tools/convert_models.py`.
- Commits use the generic identity in this repo's local git config.
- Versions: patch (0.3.0 → 0.3.1) for fixes, minor (→ 0.4.0) for features or
  behaviour changes, major (1.0.0) only for a big milestone or a breaking change to
  settings, calibrations or avatar setups. Record every change under `[Unreleased]` in
  `CHANGELOG.md`; on release, rename that section to the version and date, bump
  `Cargo.toml`, tag `vX.Y.Z` and use the section as the release notes.
- Release zips contain `vrc-openface.exe`, `DirectML.dll`, `models/`, `README.md` and `LICENSE`;
  the updater requires the exe in the zip.
