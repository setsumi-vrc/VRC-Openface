# Changelog

All notable changes to VRC Openface are listed here, newest first.

Versions follow `MAJOR.MINOR.PATCH`:

- **Patch** (0.3.0 → 0.3.1): bug fixes and small polish. Nothing new to learn.
- **Minor** (0.3.x → 0.4.0): new features or noticeable behaviour changes.
- **Major** (0.x → 1.0.0): reserved for big milestones, such as the first version
  considered complete and stable, or a change that breaks existing settings,
  calibrations or avatar setups.

## [Unreleased]

### Added
- **Linux** (x86_64): a `VRC-Openface-x.y.z-linux-x86_64.tar.gz` download with the same app. It runs on the CPU (DirectML is Windows only), opens the camera through V4L2, uses the Noto (Sans and Sans CJK) or DejaVu fonts, reads avatar configs from VRChat's Proton prefix, and updates itself from that archive. Linux has no tray icon, so the close-to-tray switch is hidden there.

### Changed
- **Tongue: a new model format** ([models/README.md](models/README.md)). The crop the model reads is turned with the mouth corners, so it follows a tilted head, and reaches below the lower lip where a tongue that sticks out ends up; on a mirrored camera the chin stays at the bottom. Its size and colour (luma or RGB) come from the model itself. A model that does not fit is reported when it is imported, with the reason, instead of failing on every frame. Models made for earlier versions do not load, and no tongue model is included.
- The tongue values are no longer averaged a second time before the app's own smoothing, so they follow the tongue with less delay.
- **About a third of the memory and less CPU.** Measured on Windows with the camera dark, the app went from 690 MB to 216 MB and from 12 % to 6 % of one core minimized (20 % to 12.5 % with the window open), with 25 threads instead of 59:
  - **Auto** runs the models on the CPU. On these small models DirectML was under a millisecond faster a frame while its device took about 450 MB. With a face in view the CPU now does that work itself (about 2 ms of CPU a frame more than with DirectML); a GPU can still be chosen in Settings.
  - ONNX Runtime runs each model on one thread (a second thread cost 30 % more CPU time), without its planned buffer layout (8 MB).
  - The tongue model does not run while the lips are closed, and while no face is found the face detector runs on every third frame instead of every frame.
  - The camera's own YUV frames are converted and shrunk by the app, read as the camera delivers them. The old RGB path also made up frames: a camera that slows to 15 FPS in low light came out at 30, every frame twice through the models.
  - The window draws nothing while minimized and redraws ten times a second while another window has the focus.
  - The Japanese, Korean and Chinese fonts (about 47 MB) load only when something on screen needs them: a CJK language, the language menu, or an avatar, camera or profile name written in CJK.
- The updater picks the download for its own system. Other systems' archives are never zips, so versions up to 0.7.0 (which take the first zip of a release) stay on the Windows one.
- The README is reorganized, up to date with the current interface, and explains how to let several apps use the camera at once.

## [0.7.0] - 2026-10-26

### Fixed
- Cleaning up after an update no longer deletes every `*.old` file below the folder the app runs from (in Downloads or on the Desktop that could take files of your own): only the files the update replaced, in the app folder and `models/`.
- The beta channel offers the final release of a version it has a pre-release of (0.7.0 after 0.7.0-beta.1); pre-releases were ranked above their release.
- An update that fails while writing its files puts the previous files back instead of leaving a half-updated app.
- The check marks of the calibration steps also show when no avatar is known (the calibration is saved, but the check looked for an avatar id).
- OSCQuery discovery no longer spins a CPU core if the mDNS service stops.
- The face detector rejects scores that are not numbers instead of taking them as faces, and checks the size of the model's output.
- The tongue status in the interface also updates while no face is tracked.
- The avatar's face tracking parameters are listed by the same rule that binds them.
- The blurred preview no longer gets slightly darker from rounding.
- The note about builds without GPU support is translated.

### Changed
- Less work per frame: the avatar bindings are shared between the tracker, the OSC thread and the interface instead of copied (the OSC thread copied them 100 times a second); interface texts, channel names and mirror sides are looked up in tables built once; the OSC sender no longer allocates for values it already sent and fits more values in each datagram; camera frames are converted into a buffer allocated once.
- Source tree: the interface is split into `src/app/` by part of the window, VRChat I/O lives in `src/vrchat/` and the drawing toolkit in `src/ui/`.

## [0.6.0] - 2026-10-26

### Added
- A "Default" color style, the app's own dark look with an amber accent; it is the style a first run starts with. Saved "Custom" colors are kept.
- Profiles in the Advanced view too: the strip across the top holds the calibration with its profiles on the left, then the camera centered in the space left; the search and the other tools stay in their row below.
- Reset all, in the Simple view's list and in the Advanced toolbar: sets every parameter back to its defaults (pressed twice to confirm; the gaze center is kept).
- The parameter whose editor is open is tinted and marked, so it is clear which one the editor belongs to.
- A small pencil on every parameter changed and not saved (range, override, linked sides, smoothing, source or symmetry), measured against the saved profile in use or, without one, the defaults; hovering the row lists what changed. Saving a profile clears them.
- Filters for the MediaPipe list (Face expressions): by group (brows, eyes, cheeks, nose, jaw, mouth, tongue) and only the modified ones, in the Simple and the Advanced view.
- An Unused switch next to the search in the Simple view's parameter list, off by default (separate from the Advanced one).
- Profiles in the Simple view: save the calibration of every parameter (ranges, overrides, linked sides, gaze center) together with the processing options under a name, load it again onto the current avatar, and delete it (pressed twice).
- The Simple view lists every MediaPipe parameter under the calibration, with search; clicking one opens its calibration, smoothing and symmetry.
- A Tongue button next to Cat ears, in both views (it is no longer a switch in the Advanced toolbar).
- The app version, small and light, next to the title in the top bar.

### Changed
- The Avatar panel says "Last avatar" in grey, with a tooltip, when VRChat is not connected and the avatar is the one remembered from the last session.
- The minimum window width is 1220 points.
- Delete (profile) and Reset all ask with a small yes and no instead of pressing twice.
- The camera in the Advanced view is larger and keeps the shape of the frame, with no black bars to the sides.
- The MediaPipe panel is called Face expressions, and the counts in the three panels show used over the full number (52/57) instead of used and unused.
- The side column of the Simple view is wider (460 pt, taken from the camera), so the parameter editor fits.
- In the Advanced view the camera and the calibration sit in a wide strip across the top (it can be collapsed) instead of a narrow sidebar, with a much larger preview.
- The tongue in the face landmarks is larger and easier to see in the small preview.
- Labels can no longer be selected with the mouse, so the title in the top bar does not highlight.
- The tongue drawn in the face landmarks follows the mouth: it comes out over the lower lip and bends toward where it points (sideways, up over the upper lip, down), on a tilted or mirrored face too; roll narrows it and curls its edges in.

### Fixed
- The size of the app data in Settings is shown with three decimals, so a small folder no longer reads 0 MB.
- The color buttons of the Custom style opened a picker that closed the menu with it; the picker now opens inside the menu.

## [0.5.0] - 2026-10-26

### Added
- A pink tongue in the camera preview's face landmarks while Tongue is on: its length follows
  tongueOut, it leans with left and right, lifts with up and narrows with roll.

## [0.4.0] - 2026-10-26

### Added
- Resolution and camera rate can be set from the camera card in both views (they stay in the settings too).
- A row linked to the hovered one, scrolled out of view in another panel, scrolls into view after a short pause; this replaces moving the expressions into a block.
- Status pill in the top bar that says whether tracking works (tracking with FPS, paused, no
  face, starting, camera or model error) and a Pause / Resume button next to it.
- Calibrate card with the two setup steps, a line saying how to do each, and a check mark once
  the current avatar has it saved.
- Settings are grouped into Connection, Interface, Camera, Processing, Updates and Data, with a
  close button; the version moved to the bottom. The GitHub and X links are in the top bar, next to the language.

### Changed
- The toolbar above the tables got a rework: a search box that lights up while typing, switches that
  slide and toggle from their label, a Symmetric face chip with an icon, a Smoothing slider with a
  marker at its value and a Reset button, all 32 points high.
- The debug performance view opens as its own window instead of floating inside the main one, with the title and window controls in its own bar like the main window.
- The MediaPipe and Tongue boxes and the Processing devices label in the settings have hover help.
- Two views, switched in the top bar and remembered: Simple (the camera as large as the window
  allows, next to the calibration) and Advanced (the camera and calibration in a sidebar, and the
  three channel tables).
- The settings open in a centered dialog instead of a side panel; Escape or a click outside closes it.
- One "Unused" switch for all three tables, in a toolbar above them with search, Tongue, Symmetric
  face, Smoothing and Reset (they were repeated in each table's header).
- The minimum window size is 1060 x 660 points; a saved window smaller than that is enlarged at start.
- The tray menu is drawn by the app in the color scheme (status line, Open, Pause / Resume, Quit)
  instead of the plain Windows menu, and a dot on the tray icon shows the tracking state. The
  tooltip says it in words.
- The window has no system title bar: minimize, maximize and close are in the top bar, which also
  drags the window and maximizes it on a double click.
- New interface: the camera preview, its options and calibration live in a sidebar, so the three
  parameter panels get the full height of the window. Below 760 points wide the sidebar becomes a
  collapsible block above the panels.
- Rounded panels and controls, Segoe UI instead of a monospace font, and icons drawn by the app
  instead of emoji, so they follow the color scheme.
- Every parameter row shows its value as a thin bar under the name (from the middle for signed
  values); the dots and marks next to channel names are drawn shapes.
- The search box moved above the panels.
- Drop-down lists light up on hover, and scroll bars keep their own strip instead of covering the content.
- A linked row that is scrolled out of view gets an arrow at the panel edge showing which way it is.

### Fixed
- A notice wider than the MediaPipe panel (no tongue model) pushed the other panels off screen.
- The Chinese language name in the language menu mixed fonts, so its first character sat off the line.

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
  Related rows are highlighted in every panel. A linked row scrolled out of view scrolls into
  view after a short pause on the hovered row.
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

[Unreleased]: https://github.com/setsumi-vrc/VRC-Openface/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/setsumi-vrc/VRC-Openface/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/setsumi-vrc/VRC-Openface/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/setsumi-vrc/VRC-Openface/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/setsumi-vrc/VRC-Openface/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/setsumi-vrc/VRC-Openface/releases/tag/v0.1.0
