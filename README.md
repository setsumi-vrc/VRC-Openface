# VRC Openface

Webcam face tracking for VRChat, written in Rust. VRC Openface runs Google's MediaPipe
face landmarker on ONNX Runtime, turns the 52 ARKit-style blendshapes into
[VRCFaceTracking](https://github.com/benaclejames/VRCFaceTracking) Unified Expressions,
and sends them to VRChat over OSC. It needs no VRCFaceTracking install, no Python and no
OpenCV.

## Features

- MediaPipe face detection, 478-point face mesh and 52 blendshapes at camera frame rate
  (about 5 ms per frame on a desktop CPU)
- Optional DirectML GPU inference (`--features directml`)
- Unified Expressions output: base shapes plus the common v2 blends (`EyeLid`,
  `BrowExpression`, `SmileFrown`, `JawX`, `MouthX`, eye gaze, …)
- Avatar parameter discovery through VRChat's OSCQuery service. Each parameter is sent
  as a float, or binary-encoded (`Name1/2/4/8…` plus `NameNegative`) to match how the
  avatar declares it.
- **Capture neutral**: records your resting face and rescales every channel from it
- **Center gaze**: records where "straight ahead" is for webcam eye tracking
- Per-channel gain (right-click a channel), symmetric face, One-Euro smoothing
- Optional tongue tracking with a user-supplied model (see [models/README.md](models/README.md))
- Settings persist in `%APPDATA%\VRC-Openface\config\settings.json`

## Building

You need Rust 1.95 or newer (required by egui 0.37).

```sh
cargo build --release
# GPU (DirectML) inference:
cargo build --release --features directml
```

ONNX Runtime binaries are downloaded by the `ort` crate at build time. The release
binary looks for a `models` folder next to the executable, then in the working
directory, so ship `target/release/vrc-openface.exe` together with `models/`.

## Using it with VRChat

1. Enable OSC in VRChat: Action menu → Options → OSC → Enabled.
2. Start VRC Openface and pick your camera. The landmark overlay should follow your face.
3. Look at the camera with a relaxed face and press **Capture neutral**. Then look
   straight ahead and press **Center gaze**. Each capture takes about 1.5 seconds.
4. Load an avatar with VRCFaceTracking-style parameters (`FT/v2/...`). The Avatar
   panel lists every parameter that was matched.

When OSCQuery cannot find VRChat, every parameter is sent as an `FT/v2/<Name>` float
(this can be turned off in Settings). OSC goes to `127.0.0.1:9000` by default; change
it in Settings.

### Panels

- **MediaPipe**: the 52 raw channels after calibration. A green dot marks a channel
  that drives a parameter on the current avatar. Right-click a channel to change its gain.
- **Unified Expressions**: every output parameter and its current value.
- **Avatar**: the matched avatar parameters, including binary-encoded ones.

## Development

```sh
cargo test
cargo clippy --all-targets
cargo run --release --example still_image -- path/to/face.jpg   # blendshapes as JSON
cargo run --release --example camera_check -- 0 640 480 30       # camera mode and real FPS
```

To regenerate the ONNX models from Google's original `.task` bundle:

```sh
uv run tools/convert_models.py
```

## License

The code is MIT licensed. The models in `models/` are Google's MediaPipe models
(Apache 2.0); see [models/README.md](models/README.md).
