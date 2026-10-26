# Changelog

All notable changes to VRC Openface are listed here, newest first.

Versions follow `MAJOR.MINOR.PATCH`:

- **Patch** (0.3.0 → 0.3.1): bug fixes and small polish. Nothing new to learn.
- **Minor** (0.3.x → 0.4.0): new features or noticeable behaviour changes.
- **Major** (0.x → 1.0.0): reserved for big milestones, such as the first version
  considered complete and stable, or a change that breaks existing settings,
  calibrations or avatar setups.

## [Unreleased]

## [0.3.0] - 2026-10-26

### Added
- System tray icon (Windows). Closing the window hides it to the tray while tracking and
  OSC keep running; click the icon or choose Open to bring it back, Quit to exit.
  The "Close to tray" setting turns that off, so closing quits.
- Pause in the tray menu stops sending tracking, like the FT switch (neutral face, tracking
  inactive); the check mark follows the FT switch.
- Seven more interface languages: Spanish, Portuguese, French, German, Italian, Russian and
  Simplified Chinese, next to English, Japanese and Korean. The language is chosen from a
  dropdown with a flag per language.
- GitHub and X links next to the title.
- Links between the parameter panels. Hovering a MediaPipe channel shows the Unified
  Expressions it feeds, with lines to them and on to the avatar parameters they drive.
  Hovering a Unified Expression or an avatar parameter draws lines back to its sources.
  Related rows are highlighted in every panel. When some expressions are scrolled out of
  view, they move into one block in the list next to the channel, always fully visible.
- Hover the blurred camera preview for a moment to see the camera unblurred; it blurs
  again after 5 minutes.
- Narrow layout: when the window (or the space beside the settings panel) is narrow,
  Unified Expressions and Avatar share one column and MediaPipe shows one channel per line.
- App icon for the window, the taskbar and `vrc-openface.exe`.
- This changelog.

### Changed
- Debug performance shows much more: latency bars per path (MediaPipe → OSC, Tongue → OSC,
  tables, preview) split into tracker, core, interface and waiting time; a graph of the
  totals; memory now / change / trend for private bytes and working set; and averages for
  every stage grouped by camera, tracker, core and interface.
- The color scheme button opens a dropdown of schemes (with the custom colors below it),
  and new installs start with the Custom scheme.
- The smoothing control in the MediaPipe panel is a slider box instead of an editable number.
- New look for the parameter panels: dashed outlines, table rows with separators, each
  MediaPipe group in its own box, and wider gaps between the panels.
- Expanded binary parameters list each bit as its own row with ON / OFF.
- Settings show each processing stage in a roomier box with its device and time; the
  device of each stage is picked right in its box.
- Bigger settings and color scheme buttons, with hover highlights.
- The toolbar wraps onto a second line instead of cutting buttons off, and the camera
  preview takes less height in small windows.
- The CJK font of the selected language is used for the whole line, so Han characters
  no longer mix fonts with different baselines.

### Fixed
- Links (GitHub, X, the Unified Expressions reference, Releases) did nothing when clicked.
- Panel headers overlapped when the window was narrow or the settings panel was open.
  Titles are now shortened with "…", and the controls move to a second line when needed.
- The MediaPipe panel slid off the left edge of the window when its header did not fit.
- Settings labels were cut off on the left because some rows (processing devices,
  tongue model) were wider than the settings panel.
- Long parameter names ran into their values; they are now shortened to fit.

## [0.2.0] - 2026-10-26

### Added
- Interpolated 100 Hz OSC output, sent as bundles so binary parameters update together.
  Float and binary versions of a parameter are both sent.
- Full 116-parameter Unified Expressions mapping with friendly names, and a geometric
  (mesh) frown estimate.
- Per-avatar calibration: neutral capture with undo, gaze centre, range, override,
  linked left/right, per-group symmetry and per-channel smoothing.
- Avatar name and parameters from VRChat's avatar config files and the receive port,
  in addition to OSCQuery.
- Settings panel: ports, interface scale, sleep when idle, updates from GitHub
  releases, debug performance, preview blur, processing devices and erase app data.
- DirectML GPU inference by default, with CPU fallback.
- Tongue model importer, cat ears, colour schemes, and English / 日本語 / 한국어.

## [0.1.0] - 2026-10-25

### Added
- First release: webcam capture, MediaPipe face landmarks and blendshapes on ONNX
  Runtime, Unified Expressions mapping and OSC output to VRChat.

[Unreleased]: https://github.com/setsumi-vrc/VRC-Openface/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/setsumi-vrc/VRC-Openface/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/setsumi-vrc/VRC-Openface/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/setsumi-vrc/VRC-Openface/releases/tag/v0.1.0
