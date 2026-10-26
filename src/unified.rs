//! MediaPipe (ARKit-style) blendshapes -> VRCFaceTracking Unified Expressions.
//!
//! Base Unified Expressions are mapped one-to-one where MediaPipe has an equivalent,
//! and the common v2 "simplified" parameters (EyeLid, BrowExpression, SmileFrown, JawX,
//! ...) are blended from the same channels. Every parameter records the channels it
//! depends on so the UI can tell which channels an avatar actually uses.

use crate::channels::{self, COUNT};
use crate::processing::Gaze;

#[derive(Clone, Copy)]
pub enum Formula {
    /// Mean of the listed channels, range [0, 1].
    Avg(&'static [&'static str]),
    /// mean(positive) - mean(negative), range [-1, 1].
    Diff(&'static [&'static str], &'static [&'static str]),
    /// Eyelid openness with widen headroom: (1 - blink) * 0.75 + wide * 0.25.
    EyeLid(&'static [&'static str], &'static [&'static str]),
    Gaze(GazeAxis),
}

#[derive(Clone, Copy)]
pub enum GazeAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    X,
    Y,
}

pub struct Param {
    pub name: &'static str,
    pub formula: Formula,
}

impl Param {
    pub fn dependencies(&self) -> Vec<usize> {
        let names: Vec<&str> = match self.formula {
            Formula::Avg(a) => a.to_vec(),
            Formula::Diff(a, b) | Formula::EyeLid(a, b) => a.iter().chain(b).copied().collect(),
            Formula::Gaze(axis) => {
                let left = ["eyeLookInLeft", "eyeLookOutLeft", "eyeLookUpLeft", "eyeLookDownLeft"];
                let right = ["eyeLookInRight", "eyeLookOutRight", "eyeLookUpRight", "eyeLookDownRight"];
                match axis {
                    GazeAxis::LeftX | GazeAxis::LeftY => left.to_vec(),
                    GazeAxis::RightX | GazeAxis::RightY => right.to_vec(),
                    GazeAxis::X | GazeAxis::Y => left.iter().chain(&right).copied().collect(),
                }
            }
        };
        names.into_iter().map(channels::index).collect()
    }

    pub fn evaluate(&self, v: &[f32; COUNT], gaze: Gaze) -> f32 {
        let mean = |names: &[&str]| names.iter().map(|n| v[channels::index(n)]).sum::<f32>() / names.len() as f32;
        match self.formula {
            Formula::Avg(a) => mean(a),
            Formula::Diff(a, b) => (mean(a) - mean(b)).clamp(-1.0, 1.0),
            Formula::EyeLid(blink, wide) => ((1.0 - mean(blink)) * 0.75 + mean(wide) * 0.25).clamp(0.0, 1.0),
            Formula::Gaze(axis) => match axis {
                GazeAxis::LeftX => gaze.left_x,
                GazeAxis::LeftY => gaze.left_y,
                GazeAxis::RightX => gaze.right_x,
                GazeAxis::RightY => gaze.right_y,
                GazeAxis::X => (gaze.left_x + gaze.right_x) * 0.5,
                GazeAxis::Y => (gaze.left_y + gaze.right_y) * 0.5,
            },
        }
    }
}

const fn avg(name: &'static str, channels: &'static [&'static str]) -> Param {
    Param { name, formula: Formula::Avg(channels) }
}

const fn diff(name: &'static str, pos: &'static [&'static str], neg: &'static [&'static str]) -> Param {
    Param { name, formula: Formula::Diff(pos, neg) }
}

const fn gaze(name: &'static str, axis: GazeAxis) -> Param {
    Param { name, formula: Formula::Gaze(axis) }
}

const BROW_UP_R: &[&str] = &["browInnerUp", "browOuterUpRight"];
const BROW_UP_L: &[&str] = &["browInnerUp", "browOuterUpLeft"];
const SMILE: &[&str] = &["mouthSmileLeft", "mouthSmileRight"];
const FROWN: &[&str] = &["mouthFrownLeft", "mouthFrownRight"];

pub const PARAMS: &[Param] = &[
    // Eyes
    gaze("EyeLeftX", GazeAxis::LeftX),
    gaze("EyeLeftY", GazeAxis::LeftY),
    gaze("EyeRightX", GazeAxis::RightX),
    gaze("EyeRightY", GazeAxis::RightY),
    gaze("EyeX", GazeAxis::X),
    gaze("EyeY", GazeAxis::Y),
    Param { name: "EyeLidLeft", formula: Formula::EyeLid(&["eyeBlinkLeft"], &["eyeWideLeft"]) },
    Param { name: "EyeLidRight", formula: Formula::EyeLid(&["eyeBlinkRight"], &["eyeWideRight"]) },
    Param {
        name: "EyeLid",
        formula: Formula::EyeLid(&["eyeBlinkLeft", "eyeBlinkRight"], &["eyeWideLeft", "eyeWideRight"]),
    },
    avg("EyeSquintLeft", &["eyeSquintLeft"]),
    avg("EyeSquintRight", &["eyeSquintRight"]),
    avg("EyeSquint", &["eyeSquintLeft", "eyeSquintRight"]),
    avg("EyeWideLeft", &["eyeWideLeft"]),
    avg("EyeWideRight", &["eyeWideRight"]),
    avg("EyeWide", &["eyeWideLeft", "eyeWideRight"]),
    // Brows
    avg("BrowDown", &["browDownLeft", "browDownRight"]),
    avg("BrowDownLeft", &["browDownLeft"]),
    avg("BrowDownRight", &["browDownRight"]),
    diff("BrowExpression", &["browInnerUp", "browOuterUpLeft", "browOuterUpRight"], &["browDownLeft", "browDownRight"]),
    diff("BrowExpressionLeft", BROW_UP_L, &["browDownLeft"]),
    diff("BrowExpressionRight", BROW_UP_R, &["browDownRight"]),
    avg("BrowInnerUp", &["browInnerUp"]),
    avg("BrowInnerUpLeft", &["browInnerUp"]),
    avg("BrowInnerUpRight", &["browInnerUp"]),
    avg("BrowLowererLeft", &["browDownLeft"]),
    avg("BrowLowererRight", &["browDownRight"]),
    avg("BrowOuterUp", &["browOuterUpLeft", "browOuterUpRight"]),
    avg("BrowOuterUpLeft", &["browOuterUpLeft"]),
    avg("BrowOuterUpRight", &["browOuterUpRight"]),
    avg("BrowPinchLeft", &["browDownLeft"]),
    avg("BrowPinchRight", &["browDownRight"]),
    avg("BrowUp", &["browInnerUp", "browOuterUpLeft", "browOuterUpRight"]),
    avg("BrowUpLeft", BROW_UP_L),
    avg("BrowUpRight", BROW_UP_R),
    // Cheeks and nose
    avg("CheekPuffLeft", &["cheekPuff"]),
    avg("CheekPuffRight", &["cheekPuff"]),
    avg("CheekPuffSuck", &["cheekPuff"]),
    avg("CheekPuffSuckLeft", &["cheekPuff"]),
    avg("CheekPuffSuckRight", &["cheekPuff"]),
    avg("CheekSquint", &["cheekSquintLeft", "cheekSquintRight"]),
    avg("CheekSquintLeft", &["cheekSquintLeft"]),
    avg("CheekSquintRight", &["cheekSquintRight"]),
    avg("NoseSneer", &["noseSneerLeft", "noseSneerRight"]),
    avg("NoseSneerLeft", &["noseSneerLeft"]),
    avg("NoseSneerRight", &["noseSneerRight"]),
    // Jaw
    avg("JawOpen", &["jawOpen"]),
    avg("JawForward", &["jawForward"]),
    avg("JawLeft", &["jawLeft"]),
    avg("JawRight", &["jawRight"]),
    diff("JawX", &["jawRight"], &["jawLeft"]),
    avg("MouthClosed", &["mouthClose"]),
    // Lips
    avg("LipFunnel", &["mouthFunnel"]),
    avg("LipFunnelLower", &["mouthFunnel"]),
    avg("LipFunnelLowerLeft", &["mouthFunnel"]),
    avg("LipFunnelLowerRight", &["mouthFunnel"]),
    avg("LipFunnelUpper", &["mouthFunnel"]),
    avg("LipFunnelUpperLeft", &["mouthFunnel"]),
    avg("LipFunnelUpperRight", &["mouthFunnel"]),
    avg("LipPucker", &["mouthPucker"]),
    avg("LipPuckerLowerLeft", &["mouthPucker"]),
    avg("LipPuckerLowerRight", &["mouthPucker"]),
    avg("LipPuckerUpperLeft", &["mouthPucker"]),
    avg("LipPuckerUpperRight", &["mouthPucker"]),
    avg("LipPuckerLeft", &["mouthPucker"]),
    avg("LipPuckerRight", &["mouthPucker"]),
    avg("LipSuck", &["mouthRollUpper", "mouthRollLower"]),
    avg("LipSuckLower", &["mouthRollLower"]),
    avg("LipSuckLowerLeft", &["mouthRollLower"]),
    avg("LipSuckLowerRight", &["mouthRollLower"]),
    avg("LipSuckUpper", &["mouthRollUpper"]),
    avg("LipSuckUpperLeft", &["mouthRollUpper"]),
    avg("LipSuckUpperRight", &["mouthRollUpper"]),
    // Mouth
    avg("MouthCornerPullLeft", &["mouthSmileLeft"]),
    avg("MouthCornerPullRight", &["mouthSmileRight"]),
    avg("MouthCornerSlantLeft", &["mouthSmileLeft"]),
    avg("MouthCornerSlantRight", &["mouthSmileRight"]),
    avg("MouthDimple", &["mouthDimpleLeft", "mouthDimpleRight"]),
    avg("MouthDimpleLeft", &["mouthDimpleLeft"]),
    avg("MouthDimpleRight", &["mouthDimpleRight"]),
    avg("MouthFrownLeft", &["mouthFrownLeft"]),
    avg("MouthFrownRight", &["mouthFrownRight"]),
    avg("MouthLowerDown", &["mouthLowerDownLeft", "mouthLowerDownRight"]),
    avg("MouthLowerDownLeft", &["mouthLowerDownLeft"]),
    avg("MouthLowerDownRight", &["mouthLowerDownRight"]),
    avg("MouthLowerLeft", &["mouthLeft"]),
    avg("MouthLowerRight", &["mouthRight"]),
    diff("MouthLowerX", &["mouthRight"], &["mouthLeft"]),
    avg("MouthOpen", &["mouthUpperUpLeft", "mouthUpperUpRight", "mouthLowerDownLeft", "mouthLowerDownRight"]),
    avg("MouthPress", &["mouthPressLeft", "mouthPressRight"]),
    avg("MouthPressLeft", &["mouthPressLeft"]),
    avg("MouthPressRight", &["mouthPressRight"]),
    avg("MouthRaiserLower", &["mouthShrugLower"]),
    avg("MouthRaiserUpper", &["mouthShrugUpper"]),
    avg("MouthSadLeft", &["mouthFrownLeft"]),
    avg("MouthSadRight", &["mouthFrownRight"]),
    avg("MouthSmileLeft", &["mouthSmileLeft"]),
    avg("MouthSmileRight", &["mouthSmileRight"]),
    avg("MouthStretch", &["mouthStretchLeft", "mouthStretchRight"]),
    avg("MouthStretchLeft", &["mouthStretchLeft"]),
    avg("MouthStretchRight", &["mouthStretchRight"]),
    avg("MouthUpperLeft", &["mouthLeft"]),
    avg("MouthUpperRight", &["mouthRight"]),
    avg("MouthUpperUp", &["mouthUpperUpLeft", "mouthUpperUpRight"]),
    avg("MouthUpperUpLeft", &["mouthUpperUpLeft"]),
    avg("MouthUpperUpRight", &["mouthUpperUpRight"]),
    diff("MouthUpperX", &["mouthRight"], &["mouthLeft"]),
    diff("MouthX", &["mouthRight"], &["mouthLeft"]),
    diff("SmileFrown", SMILE, FROWN),
    diff("SmileFrownLeft", &["mouthSmileLeft"], &["mouthFrownLeft"]),
    diff("SmileFrownRight", &["mouthSmileRight"], &["mouthFrownRight"]),
    diff("SmileSad", SMILE, &["mouthFrownLeft", "mouthFrownRight", "mouthStretchLeft", "mouthStretchRight"]),
    diff("SmileSadLeft", &["mouthSmileLeft"], &["mouthFrownLeft", "mouthStretchLeft"]),
    diff("SmileSadRight", &["mouthSmileRight"], &["mouthFrownRight", "mouthStretchRight"]),
    // Tongue
    avg("TongueOut", &["tongueOut"]),
    avg("TongueDown", &["tongueDown"]),
    avg("TongueLeft", &["tongueLeft"]),
    avg("TongueRight", &["tongueRight"]),
    avg("TongueRoll", &["tongueRoll"]),
    avg("TongueUp", &["tongueUp"]),
    diff("TongueX", &["tongueRight"], &["tongueLeft"]),
    diff("TongueY", &["tongueUp"], &["tongueDown"]),
];

pub fn evaluate_all(v: &[f32; COUNT], gaze: Gaze) -> Vec<f32> {
    PARAMS.iter().map(|p| p.evaluate(v, gaze)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dependency_resolves() {
        for p in PARAMS {
            assert!(!p.dependencies().is_empty(), "{}", p.name);
        }
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = PARAMS.iter().map(|p| p.name).collect();
        names.sort();
        let len = names.len();
        names.dedup();
        assert_eq!(len, names.len());
    }

    #[test]
    fn eyelid_is_three_quarters_at_rest() {
        let v = [0.0; COUNT];
        let lid = PARAMS.iter().find(|p| p.name == "EyeLid").unwrap();
        assert!((lid.evaluate(&v, Gaze::default()) - 0.75).abs() < 1e-6);
    }
}
