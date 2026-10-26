# Models

| File | Source network | Input | Output |
| --- | --- | --- | --- |
| `face_detector.onnx` | BlazeFace short range | `[1, 128, 128, 3]` RGB in [-1, 1] | `[1, 896, 16]` boxes, `[1, 896, 1]` logits |
| `face_landmarks_detector.onnx` | Face mesh v2 | `[N, 256, 256, 3]` RGB in [0, 1] | `[N, 1, 1, 1434]` landmarks, `[N, 1, 1, 1]` presence logit, `[N, 1]` |
| `face_blendshapes.onnx` | Blendshape MLP-Mixer | `[1, 146, 2]` landmark xy (pixels) | `[52]` scores |

These are Google's MediaPipe **Face Landmarker** models
(`face_landmarker.task`, float16, version 1, SHA-256
`64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff`) converted from
TFLite to ONNX with tf2onnx 1.17.0, opset 17. `tools/convert_models.py` reproduces
them.

The original models are © Google LLC and distributed under the
[Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0). See the
[model card](https://storage.googleapis.com/mediapipe-assets/MediaPipe%20BlazeFace%20Model%20Card%20(Short%20Range).pdf)
and the [MediaPipe Face Landmarker documentation](https://ai.google.dev/edge/mediapipe/solutions/vision/face_landmarker)
for intended use and limitations.

## Tongue model

No tongue model is included. (A build made with a `models/tongue_model.onnx` present compiles that file in and uses it
when none is imported or placed next to the executable.) Use **Import tongue model…** (Settings, or the hint
in the MediaPipe panel when Tongue is on) to pick a compatible ONNX file. It is copied
to `%APPDATA%\VRC-Openface\config\models\tongue_model.onnx`, and a
`models/tongue_model.onnx` next to the executable also works. A compatible model has:

- input `[1, C, S, S]` float: a square crop of the mouth, turned with the mouth corners and
  reaching below the lower lip, with C = 1 (luma) or 3 (RGB) planes in [0, 1]. S is read from
  the model; a model that leaves it open gets 112.
- output: six floats in [0, 1], `tongueOut, tongueLeft, tongueRight, tongueUp, tongueDown, tongueRoll`

`tongue_model.onnx` is git-ignored so third-party weights are not committed by accident.
