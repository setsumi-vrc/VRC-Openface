# /// script
# requires-python = "==3.12.*"
# dependencies = ["tensorflow-cpu==2.17.*", "tf2onnx==1.17.0", "numpy<2", "protobuf<4"]
# ///
"""Rebuild models/*.onnx from Google's official MediaPipe face landmarker bundle.

    uv run tools/convert_models.py

Downloads face_landmarker.task, checks its SHA-256, extracts the three TFLite
networks and converts each one to ONNX (opset 17) with tf2onnx.
"""

import hashlib
import io
import subprocess
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path

URL = "https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task"
SHA256 = "64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff"
MODELS = ("face_detector", "face_landmarks_detector", "face_blendshapes")


def main() -> None:
    out_dir = Path(__file__).resolve().parent.parent / "models"
    out_dir.mkdir(exist_ok=True)

    print(f"downloading {URL}")
    data = urllib.request.urlopen(URL).read()
    digest = hashlib.sha256(data).hexdigest()
    if digest != SHA256:
        sys.exit(f"unexpected task checksum {digest}")

    with tempfile.TemporaryDirectory() as tmp:
        zipfile.ZipFile(io.BytesIO(data)).extractall(tmp)
        for name in MODELS:
            target = out_dir / f"{name}.onnx"
            print(f"converting {name} -> {target}")
            subprocess.run(
                [sys.executable, "-m", "tf2onnx.convert", "--tflite", str(Path(tmp) / f"{name}.tflite"),
                 "--output", str(target), "--opset", "17"],
                check=True,
            )


if __name__ == "__main__":
    main()
