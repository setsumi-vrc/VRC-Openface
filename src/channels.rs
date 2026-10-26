//! Catalogue of tracked channels: the 52 MediaPipe blendshapes followed by the six
//! optional tongue channels.

use crate::face::TONGUE_CHANNELS;

pub const MEDIAPIPE: [&str; 52] = [
    "_neutral",
    "browDownLeft",
    "browDownRight",
    "browInnerUp",
    "browOuterUpLeft",
    "browOuterUpRight",
    "cheekPuff",
    "cheekSquintLeft",
    "cheekSquintRight",
    "eyeBlinkLeft",
    "eyeBlinkRight",
    "eyeLookDownLeft",
    "eyeLookDownRight",
    "eyeLookInLeft",
    "eyeLookInRight",
    "eyeLookOutLeft",
    "eyeLookOutRight",
    "eyeLookUpLeft",
    "eyeLookUpRight",
    "eyeSquintLeft",
    "eyeSquintRight",
    "eyeWideLeft",
    "eyeWideRight",
    "jawForward",
    "jawLeft",
    "jawOpen",
    "jawRight",
    "mouthClose",
    "mouthDimpleLeft",
    "mouthDimpleRight",
    "mouthFrownLeft",
    "mouthFrownRight",
    "mouthFunnel",
    "mouthLeft",
    "mouthLowerDownLeft",
    "mouthLowerDownRight",
    "mouthPressLeft",
    "mouthPressRight",
    "mouthPucker",
    "mouthRight",
    "mouthRollLower",
    "mouthRollUpper",
    "mouthShrugLower",
    "mouthShrugUpper",
    "mouthSmileLeft",
    "mouthSmileRight",
    "mouthStretchLeft",
    "mouthStretchRight",
    "mouthUpperUpLeft",
    "mouthUpperUpRight",
    "noseSneerLeft",
    "noseSneerRight",
];

pub const COUNT: usize = MEDIAPIPE.len() + TONGUE_CHANNELS.len();

pub fn name(index: usize) -> &'static str {
    if index < MEDIAPIPE.len() { MEDIAPIPE[index] } else { TONGUE_CHANNELS[index - MEDIAPIPE.len()] }
}

pub fn index(name: &str) -> usize {
    (0..COUNT).find(|&i| self::name(i) == name).unwrap_or_else(|| panic!("unknown channel {name}"))
}

pub fn is_tongue(index: usize) -> bool {
    index >= MEDIAPIPE.len()
}

pub fn is_eye_look(index: usize) -> bool {
    name(index).starts_with("eyeLook")
}

/// Left/right pairs averaged by "Symmetric face". The horizontal gaze pairs couple the
/// inward look of one eye with the outward look of the other.
pub const SYMMETRY: [(&str, &str); 20] = [
    ("browDownLeft", "browDownRight"),
    ("browOuterUpLeft", "browOuterUpRight"),
    ("cheekSquintLeft", "cheekSquintRight"),
    ("eyeBlinkLeft", "eyeBlinkRight"),
    ("eyeLookDownLeft", "eyeLookDownRight"),
    ("eyeLookUpLeft", "eyeLookUpRight"),
    ("eyeLookInLeft", "eyeLookOutRight"),
    ("eyeLookOutLeft", "eyeLookInRight"),
    ("eyeSquintLeft", "eyeSquintRight"),
    ("eyeWideLeft", "eyeWideRight"),
    ("jawLeft", "jawRight"),
    ("mouthDimpleLeft", "mouthDimpleRight"),
    ("mouthFrownLeft", "mouthFrownRight"),
    ("mouthLowerDownLeft", "mouthLowerDownRight"),
    ("mouthPressLeft", "mouthPressRight"),
    ("mouthSmileLeft", "mouthSmileRight"),
    ("mouthStretchLeft", "mouthStretchRight"),
    ("mouthUpperUpLeft", "mouthUpperUpRight"),
    ("noseSneerLeft", "noseSneerRight"),
    ("tongueLeft", "tongueRight"),
];

/// Human readable label, e.g. `eyeLookInLeft` -> `Eye Look In (Left)`.
pub fn label(index: usize) -> String {
    let raw = name(index);
    if raw == "_neutral" {
        return "Neutral".into();
    }
    let (base, side) = if let Some(b) = raw.strip_suffix("Left") {
        (b, Some("Left"))
    } else if let Some(b) = raw.strip_suffix("Right") {
        (b, Some("Right"))
    } else {
        (raw, None)
    };
    let mut words = String::new();
    for (i, c) in base.chars().enumerate() {
        if i == 0 {
            words.extend(c.to_uppercase());
        } else {
            if c.is_uppercase() {
                words.push(' ');
            }
            words.push(c);
        }
    }
    match side {
        Some(side) => format!("{words} ({side})"),
        None => words,
    }
}

/// Display groups for the channel panel: rows of (left, right) or a single channel.
pub fn display_rows() -> Vec<Vec<(usize, Option<usize>)>> {
    let groups: [&[&str]; 8] = [
        &["browDown", "browInnerUp", "browOuterUp"],
        &["eyeBlink", "eyeSquint", "eyeWide"],
        &["eyeLookDown", "eyeLookIn", "eyeLookOut", "eyeLookUp"],
        &["cheekPuff", "cheekSquint", "noseSneer"],
        &["jawOpen", "jawForward", "jaw", "mouthClose"],
        &["mouthSmile", "mouthFrown", "mouthDimple", "mouthStretch", "mouthPress", "mouthUpperUp", "mouthLowerDown"],
        &[
            "mouthFunnel",
            "mouthPucker",
            "mouth",
            "mouthRollUpper",
            "mouthRollLower",
            "mouthShrugUpper",
            "mouthShrugLower",
        ],
        &["tongueOut", "tongue", "tongueUp", "tongueDown", "tongueRoll"],
    ];
    let find = |n: String| (0..COUNT).find(|&i| name(i) == n);
    groups
        .iter()
        .map(|group| {
            group
                .iter()
                .filter_map(|base| match (find(format!("{base}Left")), find(format!("{base}Right"))) {
                    (Some(l), r) => Some((l, r)),
                    _ => find(base.to_string()).map(|i| (i, None)),
                })
                .collect()
        })
        .collect()
}
