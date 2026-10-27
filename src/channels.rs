//! Catalogue of tracked channels: the 52 MediaPipe blendshapes followed by the six
//! optional tongue channels.

use std::sync::LazyLock;

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

pub const fn name(index: usize) -> &'static str {
    if index < MEDIAPIPE.len() { MEDIAPIPE[index] } else { TONGUE_CHANNELS[index - MEDIAPIPE.len()] }
}

/// `str` equality usable in constants.
const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Position of the channel called `name`.
pub const fn find(name: &str) -> Option<usize> {
    let mut i = 0;
    while i < COUNT {
        if same(self::name(i), name) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Position of a channel that must exist. Use it in a `const` (or `const { }` block) so a
/// misspelled name fails the build instead of panicking at run time.
pub const fn index(name: &str) -> usize {
    match find(name) {
        Some(i) => i,
        None => panic!("unknown channel"),
    }
}

pub fn is_tongue(index: usize) -> bool {
    index >= MEDIAPIPE.len()
}

pub fn is_eye_look(index: usize) -> bool {
    name(index).starts_with("eyeLook")
}

/// Symmetry groups: each pair is averaged when its group is symmetric. The horizontal
/// gaze group couples the inward look of one eye with the outward look of the other.
pub const SYMMETRY_GROUPS: [(&str, &[(&str, &str)]); 19] = [
    ("browDown", &[("browDownLeft", "browDownRight")]),
    ("browOuterUp", &[("browOuterUpLeft", "browOuterUpRight")]),
    ("cheekSquint", &[("cheekSquintLeft", "cheekSquintRight")]),
    ("eyeBlink", &[("eyeBlinkLeft", "eyeBlinkRight")]),
    ("eyeLookDown", &[("eyeLookDownLeft", "eyeLookDownRight")]),
    ("eyeLookUp", &[("eyeLookUpLeft", "eyeLookUpRight")]),
    ("eyeLookHorizontal", &[("eyeLookInLeft", "eyeLookOutRight"), ("eyeLookOutLeft", "eyeLookInRight")]),
    ("eyeSquint", &[("eyeSquintLeft", "eyeSquintRight")]),
    ("eyeWide", &[("eyeWideLeft", "eyeWideRight")]),
    ("jaw", &[("jawLeft", "jawRight")]),
    ("mouthDimple", &[("mouthDimpleLeft", "mouthDimpleRight")]),
    ("mouthFrown", &[("mouthFrownLeft", "mouthFrownRight")]),
    ("mouthLowerDown", &[("mouthLowerDownLeft", "mouthLowerDownRight")]),
    ("mouthPress", &[("mouthPressLeft", "mouthPressRight")]),
    ("mouthSmile", &[("mouthSmileLeft", "mouthSmileRight")]),
    ("mouthStretch", &[("mouthStretchLeft", "mouthStretchRight")]),
    ("mouthUpperUp", &[("mouthUpperUpLeft", "mouthUpperUpRight")]),
    ("noseSneer", &[("noseSneerLeft", "noseSneerRight")]),
    ("tongue", &[("tongueLeft", "tongueRight")]),
];

/// Groups left out of the global "Symmetric face" switch so sideways jaw and tongue
/// movement stays independent.
pub const ASYMMETRIC_BY_DEFAULT: [&str; 2] = ["jaw", "tongue"];

/// Groups of the filter: the text key of each and the channel name prefix that belongs to it.
pub const CATEGORIES: [(&str, &str); 7] = [
    ("cat_brows", "brow"),
    ("cat_eyes", "eye"),
    ("cat_cheeks", "cheek"),
    ("cat_nose", "nose"),
    ("cat_jaw", "jaw"),
    ("cat_mouth", "mouth"),
    ("tongue", "tongue"),
];

/// Position of the channel's group in [`CATEGORIES`].
pub fn category(index: usize) -> Option<usize> {
    let n = name(index);
    CATEGORIES.iter().position(|(_, prefix)| n.starts_with(prefix))
}

/// Symmetry group a channel belongs to.
pub fn symmetry_group(index: usize) -> Option<&'static str> {
    let n = name(index);
    SYMMETRY_GROUPS.iter().find(|(_, pairs)| pairs.iter().any(|(a, b)| *a == n || *b == n)).map(|(g, _)| *g)
}

/// The other side of a Left/Right channel, e.g. `eyeBlinkLeft` -> `eyeBlinkRight`.
pub fn mirror(index: usize) -> Option<usize> {
    // Looked up by name once; the interface asks for every row in every frame.
    static MIRRORS: LazyLock<[Option<usize>; COUNT]> = LazyLock::new(|| {
        std::array::from_fn(|i| {
            let n = name(i);
            let other = match (n.strip_suffix("Left"), n.strip_suffix("Right")) {
                (Some(base), _) => format!("{base}Right"),
                (_, Some(base)) => format!("{base}Left"),
                _ => return None,
            };
            find(&other)
        })
    });
    MIRRORS.get(index).copied().flatten()
}

/// Human readable label, e.g. `eyeLookInLeft` -> `Eye Look In (Left)`.
pub fn label(index: usize) -> String {
    let raw = name(index);
    if raw == "_neutral" {
        return "Neutral".into();
    }
    let (base, side) = match (raw.strip_suffix("Left"), raw.strip_suffix("Right")) {
        (Some(b), _) => (b, Some("Left")),
        (_, Some(b)) => (b, Some("Right")),
        _ => (raw, None),
    };
    let mut words = String::with_capacity(base.len() + 8);
    for (i, c) in base.chars().enumerate() {
        if i == 0 {
            words.extend(c.to_uppercase());
            continue;
        }
        if c.is_uppercase() {
            words.push(' ');
        }
        words.push(c);
    }
    match side {
        Some(side) => format!("{words} ({side})"),
        None => words,
    }
}

/// One line of the channel panel: a channel, or a left channel and its right side.
pub type DisplayRow = (usize, Option<usize>);

/// Display groups for the channel panel, built once.
pub fn display_rows() -> &'static [Vec<DisplayRow>] {
    static ROWS: LazyLock<Vec<Vec<DisplayRow>>> = LazyLock::new(build_display_rows);
    &ROWS
}

fn build_display_rows() -> Vec<Vec<DisplayRow>> {
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
    groups
        .iter()
        .map(|group| {
            group
                .iter()
                .filter_map(|base| match (find(&format!("{base}Left")), find(&format!("{base}Right"))) {
                    (Some(l), r) => Some((l, r)),
                    _ => find(base).map(|i| (i, None)),
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_and_mirror() {
        assert_eq!(find("eyeBlinkLeft").map(name), Some("eyeBlinkLeft"));
        assert_eq!(find("nope"), None);
        let (l, r) = (index("eyeBlinkLeft"), index("eyeBlinkRight"));
        assert_eq!((mirror(l), mirror(r)), (Some(r), Some(l)));
        assert_eq!(mirror(index("jawOpen")), None);
        assert_eq!(label(index("eyeLookInLeft")), "Eye Look In (Left)");
        // Every channel but the neutral score has a place in the panel.
        let shown: usize = display_rows().iter().flatten().map(|(_, r)| 1 + usize::from(r.is_some())).sum();
        assert_eq!(shown, COUNT - 1);
    }
}
