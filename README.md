<p align="center">
  <img src="assets/icon.png" width="128" alt="VRC Openface icon">
</p>

<h1 align="center">VRC Openface</h1>

<p align="center">
  <b>Webcam face tracking for VRChat.</b><br>
  One small app for Windows and Linux: your webcam in, your avatar's face out. No VRCFaceTracking, no Python, no OpenCV.
</p>

<p align="center">
  <a href="https://github.com/setsumi-vrc/VRC-Openface/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/setsumi-vrc/VRC-Openface?style=flat-square&color=ffaa3c"></a>
  <a href="https://github.com/setsumi-vrc/VRC-Openface/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/setsumi-vrc/VRC-Openface/total?style=flat-square&color=62cc8c"></a>
  <img alt="Windows and Linux" src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux-5aaaff?style=flat-square">
  <img alt="Rust" src="https://img.shields.io/badge/made%20with-Rust-dea584?style=flat-square">
  <a href="LICENSE"><img alt="MIT license" src="https://img.shields.io/badge/license-MIT-b28cff?style=flat-square"></a>
</p>

<p align="center">
  <a href="#-quick-start">Quick start</a> ·
  <a href="#-features">Features</a> ·
  <a href="#-troubleshooting">Troubleshooting</a> ·
  <a href="#-building-from-source">Building</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

---

VRC Openface runs Google's MediaPipe face landmarker on ONNX Runtime (on your GPU through
DirectML on Windows, or on the CPU), turns its 52 blendshapes into
[VRCFaceTracking](https://github.com/benaclejames/VRCFaceTracking) **Unified Expressions** and
sends them to VRChat over OSC, 100 times a second.

## ✨ Highlights

|   |   |
|---|---|
| 🎥 **Just a webcam** | 478-point face mesh, 52 expressions and optional tongue tracking from any webcam. |
| 🧩 **116 parameters** | The Unified Expressions v2 set, float and binary, matched to whatever your avatar has. |
| 🌊 **Smooth motion** | Sent at 100 Hz and blended between camera frames, so the face never steps at 30 FPS. |
| 🎯 **Per-avatar calibration** | Capture your neutral face and gaze once; fine-tune any parameter; save profiles. |
| 🔗 **See what drives what** | Hover any value to see the chain from camera to avatar, linked across three panels. |
| 🌍 **10 languages** | English, Español, Português, Français, Deutsch, Italiano, Русский, 简体中文, 日本語, 한국어. |

## 🚀 Quick start

### 1. Install

Download the latest release from
[**Releases**](https://github.com/setsumi-vrc/VRC-Openface/releases/latest), then:

<details open>
<summary><b>Windows</b></summary>

1. Download `VRC-Openface-x.y.z.zip`, extract it anywhere and run `vrc-openface.exe`.
2. Keep the `models` folder and `DirectML.dll` next to the executable.

> [!NOTE]
> The app needs the Microsoft Visual C++ 2015–2022 Redistributable (x64), which most Windows PCs already have.

</details>

<details open>
<summary><b>Linux</b></summary>

1. Download `VRC-Openface-x.y.z-linux-x86_64.tar.gz` and extract it:

   ```sh
   tar xzf VRC-Openface-*-linux-x86_64.tar.gz
   ./VRC-Openface-*/vrc-openface
   ```

2. Keep the `models` folder next to the executable.

- Works on X11 and Wayland, on distributions with glibc 2.38 or newer (Ubuntu 24.04+,
  Debian 13+, Fedora 39+, Arch and the distributions based on them).
- Tracking runs on the CPU; GPU inference (DirectML) is Windows only. The face models are small,
  so a recent CPU keeps up with a 30 FPS camera.
- The camera is opened through V4L2 (`/dev/video*`). If it is not listed, add yourself to the
  `video` group (`sudo usermod -aG video $USER`, then log in again).
- Japanese, Korean and Chinese text needs the Noto Sans CJK fonts (`fonts-noto-cjk` on
  Debian and Ubuntu, `noto-fonts-cjk` on Arch and Fedora).
- There is no tray icon on Linux: closing the window quits the app.

</details>

Later versions install from **Settings → Updates**, inside the app, on both systems.

### 2. Let several apps use the camera

> [!IMPORTANT]
> Windows normally gives a camera to **one app at a time**. If Discord, OBS, a browser or any
> other app is using your webcam, VRC Openface cannot open it (and the other way round).
> Turn on sharing so they can all use it at once:
>
> **Settings → Bluetooth & devices → Cameras →** *(your camera)* **→ Advanced camera options →
> Allow multiple apps to use the camera at the same time**
>
> This option is part of recent Windows 11 versions. Without it, close the other apps that use
> the camera before starting VRC Openface.
>
> On Linux most cameras also serve one app at a time: close the other apps that use the camera
> first.

### 3. Connect to VRChat

1. **Enable OSC in VRChat**: Action menu → Options → OSC → **Enabled**. On Linux this works the
   same with VRChat running through Steam's Proton.
2. **Pick your camera** in VRC Openface. The dots of the face mesh should follow your face.
3. **Calibrate**:
   - Relaxed face, eyes open, looking straight ahead → **Capture neutral**.
   - Looking straight ahead → **Center gaze**.

   Each capture takes about 1.5 seconds and is saved for the current avatar.
4. **Load an avatar** with VRCFaceTracking parameters (`FT/v2/...`). The Avatar panel lists every
   parameter it found; binary ones show their bits.

That's it. **Pause** in the top bar sends a neutral face and marks tracking as inactive;
closing the window keeps tracking running in the system tray.

## 🧰 Features

<details open>
<summary><b>Tracking</b></summary>

- MediaPipe face detection, 478-point face mesh and 52 blendshapes at the camera frame rate.
- GPU inference through DirectML on Windows, with an automatic fallback to the CPU. Face and
  tongue can run on different devices. Linux runs on the CPU.
- **Tongue tracking** with an imported ONNX model ([details](models/README.md)).
- A geometric (mesh) frown estimate, because the model's own frown score rarely moves.

</details>

<details open>
<summary><b>Output to VRChat</b></summary>

- **116 Unified Expressions parameters**: the base shapes plus the v2 blends (`EyeLid`,
  `BrowExpression`, `SmileFrown`, `SmileSad`, `JawX`, `MouthX`, eye gaze, …).
- OSC at **100 Hz**, interpolated between camera frames.
- Values go out as OSC bundles, so every bit of a binary parameter changes in the same VRChat
  frame.
- A parameter that exists both as a float and as binary bools (`Name1/2/4/8…` + `NameNegative`)
  gets both: the float for you, the bits for everyone else.
- **Avatar aware**: reads the current avatar's name and parameters from VRChat's OSCQuery
  service, its local avatar files (on Linux, inside VRChat's Proton prefix) and `/avatar/change`. Without any of them, every parameter is
  sent as an `FT/v2/<Name>` float (Settings can turn this off).
- **Cat ears** 🐱: drives `EarLeft` / `EarRight` (v2, −1…1) from your brows. Raised brows perk
  them up, lowered brows fold them back. Add those float parameters to your avatar to use it.

</details>

<details open>
<summary><b>Calibration</b></summary>

- **Capture neutral** (with undo) moves each channel's minimum to your resting face;
  **Center gaze** stores where straight ahead is.
- Click any parameter to set its range or a fixed value, link or split left and right, and set
  symmetry, smoothing and (for frown) the model or mesh source.
- **Profiles**: save the whole calibration and processing options under a name, load them onto
  any avatar.
- **Smoothing** (One-Euro, global or per parameter) and a **symmetric face** switch.
- Filters by group (brows, eyes, mouth, …) and a pencil on every changed parameter.

</details>

<details open>
<summary><b>Interface</b></summary>

- **Simple view** for everyday use: a large camera preview, calibration and the parameter list.
- **Advanced view** with three linked panels: MediaPipe → Unified Expressions → Avatar. Hover a
  row to see what it feeds and what feeds it.
- Camera preview blurred by default; rest the pointer on it to reveal it for 5 minutes.
- Color schemes, including your own colors, and interface scaling.
- System tray icon with a status dot, plus Open, Pause and Quit (Windows).
- Sleeps when idle, updates itself from GitHub releases, and has a debug window with latency,
  memory and timing per stage.

</details>

## 🩺 Troubleshooting

<details>
<summary><b>"Camera error" or a black preview</b></summary>

Another app is probably using the camera. Turn on
[camera sharing](#2-let-several-apps-use-the-camera), or close that app, then press the refresh
button next to the camera list.
</details>

<details>
<summary><b>The avatar does not move</b></summary>

- Check that OSC is enabled in VRChat (Action menu → Options → OSC).
- Check that tracking is not paused (the top bar says <i>Tracking</i>, not <i>Paused</i>).
- Your avatar needs VRCFaceTracking parameters (`FT/v2/...`). The Avatar panel shows which ones
  it found.
- If you changed avatar parameters, reset the OSC config of that avatar in VRChat (Action menu →
  Options → OSC → Reset Config).
</details>

<details>
<summary><b>Expressions are too weak or too strong</b></summary>

Capture neutral again with a relaxed face, then click the parameter and adjust its range. Save the
result as a profile to reuse it.
</details>

## 📁 Your data

Settings, calibrations, profiles and imported models live in `%APPDATA%\VRC-Openface\config` on
Windows and in `~/.config/vrc-openface` on Linux.
**Settings → Data → Erase all app data** removes them. Nothing is sent anywhere except OSC to
VRChat and the update check on GitHub.

## 🔧 Building from source

Requires **Rust 1.95** or newer.

```sh
cargo build --release                         # Windows, with DirectML GPU support (default)
cargo build --release --no-default-features   # CPU only; the only build on Linux
```

The `ort` crate downloads the ONNX Runtime binaries at build time. On Linux the camera bindings
also need `libclang` (`libclang-dev` on Debian and Ubuntu).

To ship a build, copy the `models/` folder with `target/release/vrc-openface.exe` and
`target/release/DirectML.dll` (Windows) or `target/release/vrc-openface` (Linux).

<details>
<summary><b>Development commands</b></summary>

```sh
cargo test
cargo clippy --all-targets
cargo run --release --example still_image -- path/to/face.jpg   # blendshapes of one image as JSON
cargo run --release --example camera_check -- 0 640 480 30       # camera mode and the real FPS
uv run tools/convert_models.py                                   # rebuild the face models in models/
uv run tools/make_icons.py                                       # rebuild the app icon in assets/
uv run tools/make_link_icons.py                                  # rebuild the GitHub and X marks
uv run tools/make_flags.py                                       # rebuild the language flags
```

`VRC_OPENFACE_DATA=<folder>` runs the app with a separate data folder, leaving your own untouched.
</details>

## 📜 License

The code is [MIT](LICENSE) licensed. The face models in `models/` are Google's MediaPipe models
(Apache 2.0); see [models/README.md](models/README.md) for them and for the tongue model.
