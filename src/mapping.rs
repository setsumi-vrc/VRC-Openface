//! Calibrated channels -> VRCFaceTracking Unified Expressions (v2) parameters.
//!
//! Each output lists the channels it reads, in the order its formula receives them, so
//! dependencies (the "used" dots in the UI) come from the same table as the values.

use crate::channels::{self, COUNT};
use crate::processing::Gaze;

#[derive(Clone, Copy)]
pub enum GazeAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    X,
    Y,
}

#[derive(Clone, Copy)]
pub enum Eval {
    F(fn(&[f32]) -> f32),
    Gaze(GazeAxis),
    /// Reserved Unified Expressions that webcam tracking cannot measure; sent as 0.
    Zero,
}

pub struct Output {
    /// Parameter name after `v2/` on the avatar.
    pub name: &'static str,
    /// Short label shown with "Friendly names".
    pub friendly: &'static str,
    pub inputs: &'static [&'static str],
    pub eval: Eval,
    pub signed: bool,
    /// Sent only while "Cat ears" is enabled.
    pub cat_ears: bool,
}

impl Output {
    pub fn label(&self, friendly: bool) -> &'static str {
        if friendly { self.friendly } else { self.name }
    }
}

const fn out(
    name: &'static str,
    friendly: &'static str,
    inputs: &'static [&'static str],
    f: fn(&[f32]) -> f32,
) -> Output {
    Output { name, friendly, inputs, eval: Eval::F(f), signed: false, cat_ears: false }
}

const fn signed(
    name: &'static str,
    friendly: &'static str,
    inputs: &'static [&'static str],
    f: fn(&[f32]) -> f32,
) -> Output {
    Output { name, friendly, inputs, eval: Eval::F(f), signed: true, cat_ears: false }
}

const fn direct(name: &'static str, friendly: &'static str, input: &'static [&'static str]) -> Output {
    out(name, friendly, input, first)
}

const fn mean_of(name: &'static str, friendly: &'static str, inputs: &'static [&'static str]) -> Output {
    out(name, friendly, inputs, mean)
}

const fn gaze(name: &'static str, inputs: &'static [&'static str], axis: GazeAxis) -> Output {
    Output { name, friendly: name, inputs, eval: Eval::Gaze(axis), signed: true, cat_ears: false }
}

const fn zero(name: &'static str) -> Output {
    Output { name, friendly: name, inputs: &[], eval: Eval::Zero, signed: false, cat_ears: false }
}

const fn ear(name: &'static str, inputs: &'static [&'static str]) -> Output {
    Output { name, friendly: name, inputs, eval: Eval::F(ear_pose), signed: true, cat_ears: true }
}

fn first(v: &[f32]) -> f32 {
    v[0]
}

fn mean(v: &[f32]) -> f32 {
    v.iter().sum::<f32>() / v.len() as f32
}

fn max2(v: &[f32]) -> f32 {
    v[0].max(v[1])
}

fn diff(v: &[f32]) -> f32 {
    v[0] - v[1]
}

fn open(v: &[f32]) -> f32 {
    1.0 - mean(v)
}

/// Eyelid openness: 0.75 is relaxed-open, blinking closes toward 0, widening opens to 1.
fn lid(blink: f32, wide: f32) -> f32 {
    if blink > 0.0 { 0.75 * (1.0 - blink) } else { 0.75 + 0.25 * wide }
}

/// Raised minus lowered brow, with the raise half inner and half outer.
fn brow_expression(inner: f32, outer: f32, down: f32) -> f32 {
    (inner * 0.5 + outer * 0.5).min(1.0) - down
}

fn brow_up(v: &[f32]) -> f32 {
    let [inner, outer_l, outer_r] = [v[0], v[1], v[2]];
    ((outer_l * 0.6 + inner * 0.4) + (outer_r * 0.6 + inner * 0.4)) * 0.5
}

/// Ears perk up with raised brows and fold back with lowered ones, in [-1, 1].
fn ear_pose(v: &[f32]) -> f32 {
    let [inner, outer, down, wide] = [v[0], v[1], v[2], v[3]];
    ((inner * 0.5 + outer * 0.5) * 1.6 + wide * 0.4 - down * 1.8).clamp(-1.0, 1.0)
}

const LOOK_L: &[&str] = &["eyeLookInLeft", "eyeLookOutLeft", "eyeLookUpLeft", "eyeLookDownLeft"];
const LOOK_R: &[&str] = &["eyeLookInRight", "eyeLookOutRight", "eyeLookUpRight", "eyeLookDownRight"];
const LOOK: &[&str] = &[
    "eyeLookInLeft",
    "eyeLookOutLeft",
    "eyeLookUpLeft",
    "eyeLookDownLeft",
    "eyeLookInRight",
    "eyeLookOutRight",
    "eyeLookUpRight",
    "eyeLookDownRight",
];

pub const OUTPUTS: &[Output] = &[
    // Eyes
    gaze("EyeLeftX", LOOK_L, GazeAxis::LeftX),
    gaze("EyeLeftY", LOOK_L, GazeAxis::LeftY),
    gaze("EyeRightX", LOOK_R, GazeAxis::RightX),
    gaze("EyeRightY", LOOK_R, GazeAxis::RightY),
    gaze("EyeX", LOOK, GazeAxis::X),
    gaze("EyeY", LOOK, GazeAxis::Y),
    out("EyeLidRight", "EyeLidRight", &["eyeBlinkRight", "eyeWideRight"], |v| lid(v[0], v[1])),
    out("EyeLidLeft", "EyeLidLeft", &["eyeBlinkLeft", "eyeWideLeft"], |v| lid(v[0], v[1])),
    out("EyeLid", "EyeLid", &["eyeBlinkLeft", "eyeWideLeft", "eyeBlinkRight", "eyeWideRight"], |v| {
        (lid(v[0], v[1]) + lid(v[2], v[3])) * 0.5
    }),
    direct("EyeSquintRight", "EyeSquintRight", &["eyeSquintRight"]),
    direct("EyeSquintLeft", "EyeSquintLeft", &["eyeSquintLeft"]),
    mean_of("EyeSquint", "EyeSquint", &["eyeSquintLeft", "eyeSquintRight"]),
    out("EyesSquint", "EyesSquint", &["eyeSquintLeft", "eyeSquintRight"], max2),
    out("EyeOpenLeft", "EyeOpenLeft", &["eyeBlinkLeft"], open),
    out("EyeOpenRight", "EyeOpenRight", &["eyeBlinkRight"], open),
    out("EyeOpen", "EyeOpen", &["eyeBlinkLeft", "eyeBlinkRight"], open),
    direct("EyeClosedLeft", "EyeClosedLeft", &["eyeBlinkLeft"]),
    direct("EyeClosedRight", "EyeClosedRight", &["eyeBlinkRight"]),
    mean_of("EyeClosed", "EyeClosed", &["eyeBlinkLeft", "eyeBlinkRight"]),
    direct("EyeWideLeft", "EyeWideLeft", &["eyeWideLeft"]),
    direct("EyeWideRight", "EyeWideRight", &["eyeWideRight"]),
    out("EyeWide", "EyeWide", &["eyeWideLeft", "eyeWideRight"], max2),
    // Brows
    direct("BrowLowererLeft", "BrowLowererLeft", &["browDownLeft"]),
    direct("BrowLowererRight", "BrowLowererRight", &["browDownRight"]),
    direct("BrowInnerUpLeft", "BrowInnerUpLeft", &["browInnerUp"]),
    direct("BrowInnerUpRight", "BrowInnerUpRight", &["browInnerUp"]),
    direct("BrowOuterUpLeft", "BrowOuterUpLeft", &["browOuterUpLeft"]),
    direct("BrowOuterUpRight", "BrowOuterUpRight", &["browOuterUpRight"]),
    out("BrowUp", "BrowUp", &["browInnerUp", "browOuterUpLeft", "browOuterUpRight"], brow_up),
    signed("BrowExpressionRight", "BrowExprRight", &["browInnerUp", "browOuterUpRight", "browDownRight"], |v| {
        brow_expression(v[0], v[1], v[2])
    }),
    signed("BrowExpressionLeft", "BrowExprLeft", &["browInnerUp", "browOuterUpLeft", "browDownLeft"], |v| {
        brow_expression(v[0], v[1], v[2])
    }),
    signed(
        "BrowExpression",
        "BrowExpression",
        &["browInnerUp", "browOuterUpLeft", "browDownLeft", "browOuterUpRight", "browDownRight"],
        |v| (brow_expression(v[0], v[1], v[2]) + brow_expression(v[0], v[3], v[4])) * 0.5,
    ),
    direct("BrowDownLeft", "BrowDownLeft", &["browDownLeft"]),
    direct("BrowDownRight", "BrowDownRight", &["browDownRight"]),
    mean_of("BrowDown", "BrowDown", &["browDownLeft", "browDownRight"]),
    direct("BrowInnerUp", "BrowInnerUp", &["browInnerUp"]),
    mean_of("BrowOuterUp", "BrowOuterUp", &["browOuterUpLeft", "browOuterUpRight"]),
    // Jaw, nose, cheeks
    direct("JawOpen", "JawOpen", &["jawOpen"]),
    direct("MouthClosed", "MouthClosed", &["mouthClose"]),
    signed("JawX", "JawX", &["jawRight", "jawLeft"], diff),
    direct("JawZ", "JawZ", &["jawForward"]),
    direct("NoseSneerLeft", "NoseSneerLeft", &["noseSneerLeft"]),
    direct("NoseSneerRight", "NoseSneerRight", &["noseSneerRight"]),
    mean_of("NoseSneer", "NoseSneer", &["noseSneerLeft", "noseSneerRight"]),
    direct("CheekSquintLeft", "CheekSquintLeft", &["cheekSquintLeft"]),
    direct("CheekSquintRight", "CheekSquintRight", &["cheekSquintRight"]),
    mean_of("CheekSquint", "CheekSquint", &["cheekSquintLeft", "cheekSquintRight"]),
    direct("CheekPuffSuckLeft", "CheekPuffLeft", &["cheekPuff"]),
    direct("CheekPuffSuckRight", "CheekPuffRight", &["cheekPuff"]),
    direct("CheekPuffSuck", "CheekPuff", &["cheekPuff"]),
    // Mouth
    direct("MouthUpperUpLeft", "MouthUpperUpLeft", &["mouthUpperUpLeft"]),
    direct("MouthUpperUpRight", "MouthUpperUpRight", &["mouthUpperUpRight"]),
    direct("MouthLowerDownLeft", "MouthLowerDownLeft", &["mouthLowerDownLeft"]),
    direct("MouthLowerDownRight", "MouthLowerDownRight", &["mouthLowerDownRight"]),
    direct("MouthCornerPullLeft", "CornerPullLeft", &["mouthSmileLeft"]),
    direct("MouthCornerPullRight", "CornerPullRight", &["mouthSmileRight"]),
    direct("MouthFrownLeft", "MouthFrownLeft", &["mouthFrownLeft"]),
    direct("MouthFrownRight", "MouthFrownRight", &["mouthFrownRight"]),
    direct("MouthStretchLeft", "StretchLeft", &["mouthStretchLeft"]),
    direct("MouthStretchRight", "StretchRight", &["mouthStretchRight"]),
    mean_of("MouthStretch", "Stretch", &["mouthStretchLeft", "mouthStretchRight"]),
    direct("MouthDimpleLeft", "DimpleLeft", &["mouthDimpleLeft"]),
    direct("MouthDimpleRight", "DimpleRight", &["mouthDimpleRight"]),
    mean_of("MouthDimple", "Dimple", &["mouthDimpleLeft", "mouthDimpleRight"]),
    direct("MouthPressLeft", "PressLeft", &["mouthPressLeft"]),
    direct("MouthPressRight", "PressRight", &["mouthPressRight"]),
    mean_of("MouthPress", "Press", &["mouthPressLeft", "mouthPressRight"]),
    direct("MouthRaiserUpper", "RaiserUpper", &["mouthShrugUpper"]),
    direct("MouthRaiserLower", "RaiserLower", &["mouthShrugLower"]),
    // Lips
    direct("LipFunnelUpperRight", "FunnelUpR", &["mouthFunnel"]),
    direct("LipFunnelUpperLeft", "FunnelUpL", &["mouthFunnel"]),
    direct("LipFunnelLowerRight", "FunnelLoR", &["mouthFunnel"]),
    direct("LipFunnelLowerLeft", "FunnelLoL", &["mouthFunnel"]),
    direct("LipFunnelUpper", "FunnelUpper", &["mouthFunnel"]),
    direct("LipFunnelLower", "FunnelLower", &["mouthFunnel"]),
    direct("LipFunnel", "LipFunnel", &["mouthFunnel"]),
    direct("LipPuckerUpperRight", "PuckerUpR", &["mouthPucker"]),
    direct("LipPuckerUpperLeft", "PuckerUpL", &["mouthPucker"]),
    direct("LipPuckerLowerRight", "PuckerLoR", &["mouthPucker"]),
    direct("LipPuckerLowerLeft", "PuckerLoL", &["mouthPucker"]),
    direct("LipPuckerUpper", "PuckerUpper", &["mouthPucker"]),
    direct("LipPuckerLower", "PuckerLower", &["mouthPucker"]),
    direct("LipPucker", "LipPucker", &["mouthPucker"]),
    direct("LipPuckerLeft", "PuckerLeft", &["mouthPucker"]),
    direct("LipPuckerRight", "PuckerRight", &["mouthPucker"]),
    direct("LipSuckUpperLeft", "SuckUpL", &["mouthRollUpper"]),
    direct("LipSuckUpperRight", "SuckUpR", &["mouthRollUpper"]),
    direct("LipSuckLowerLeft", "SuckLoL", &["mouthRollLower"]),
    direct("LipSuckLowerRight", "SuckLoR", &["mouthRollLower"]),
    direct("LipSuckUpper", "SuckUpper", &["mouthRollUpper"]),
    direct("LipSuckLower", "SuckLower", &["mouthRollLower"]),
    mean_of("LipSuck", "LipSuck", &["mouthRollUpper", "mouthRollLower"]),
    signed("MouthX", "MouthX", &["mouthRight", "mouthLeft"], diff),
    signed("MouthUpperX", "MouthUpperX", &["mouthRight", "mouthLeft"], diff),
    signed("MouthLowerX", "MouthLowerX", &["mouthRight", "mouthLeft"], diff),
    mean_of(
        "MouthOpen",
        "MouthOpen",
        &["mouthUpperUpLeft", "mouthUpperUpRight", "mouthLowerDownLeft", "mouthLowerDownRight"],
    ),
    direct("MouthSmileLeft", "SmileLeft", &["mouthSmileLeft"]),
    direct("MouthSmileRight", "SmileRight", &["mouthSmileRight"]),
    out("MouthSadLeft", "MouthSadLeft", &["mouthFrownLeft", "mouthStretchLeft"], max2),
    out("MouthSadRight", "MouthSadRight", &["mouthFrownRight", "mouthStretchRight"], max2),
    mean_of("MouthUpperUp", "UpperUp", &["mouthUpperUpLeft", "mouthUpperUpRight"]),
    mean_of("MouthLowerDown", "LowerDown", &["mouthLowerDownLeft", "mouthLowerDownRight"]),
    signed("SmileFrownRight", "SmileFrownR", &["mouthSmileRight", "mouthFrownRight"], diff),
    signed("SmileFrownLeft", "SmileFrownL", &["mouthSmileLeft", "mouthFrownLeft"], diff),
    signed(
        "SmileFrown",
        "SmileFrown",
        &["mouthSmileRight", "mouthFrownRight", "mouthSmileLeft", "mouthFrownLeft"],
        |v| ((v[0] - v[1]) + (v[2] - v[3])) * 0.5,
    ),
    signed("SmileSadRight", "SmileSadR", &["mouthSmileRight", "mouthFrownRight", "mouthStretchRight"], |v| {
        v[0] - v[1].max(v[2])
    }),
    signed("SmileSadLeft", "SmileSadL", &["mouthSmileLeft", "mouthFrownLeft", "mouthStretchLeft"], |v| {
        v[0] - v[1].max(v[2])
    }),
    signed(
        "SmileSad",
        "SmileSad",
        &[
            "mouthSmileRight",
            "mouthFrownRight",
            "mouthStretchRight",
            "mouthSmileLeft",
            "mouthFrownLeft",
            "mouthStretchLeft",
        ],
        |v| ((v[0] - v[1].max(v[2])) + (v[3] - v[4].max(v[5]))) * 0.5,
    ),
    // Tongue
    direct("TongueOut", "TongueOut", &["tongueOut"]),
    signed("TongueX", "TongueX", &["tongueRight", "tongueLeft"], diff),
    signed("TongueY", "TongueY", &["tongueUp", "tongueDown"], diff),
    direct("TongueRoll", "TongueRoll", &["tongueRoll"]),
    zero("TongueArchY"),
    zero("TongueShape"),
    zero("TongueTwistRight"),
    zero("TongueTwistLeft"),
    // Cat ears (our own addition)
    ear("EarLeft", &["browInnerUp", "browOuterUpLeft", "browDownLeft", "eyeWideLeft"]),
    ear("EarRight", &["browInnerUp", "browOuterUpRight", "browDownRight", "eyeWideRight"]),
];

/// Pre-resolved channel indices so evaluation never compares strings.
pub struct Mapper {
    indices: Vec<Vec<usize>>,
}

impl Default for Mapper {
    fn default() -> Self {
        Self::new()
    }
}

impl Mapper {
    pub fn new() -> Self {
        Self { indices: OUTPUTS.iter().map(|o| o.inputs.iter().map(|n| channels::index(n)).collect()).collect() }
    }

    pub fn dependencies(&self, output: usize) -> &[usize] {
        &self.indices[output]
    }

    pub fn evaluate(&self, values: &[f32; COUNT], gaze: Gaze) -> Vec<f32> {
        let mut scratch = Vec::with_capacity(8);
        OUTPUTS
            .iter()
            .zip(&self.indices)
            .map(|(output, indices)| {
                let value = match output.eval {
                    Eval::F(f) => {
                        scratch.clear();
                        scratch.extend(indices.iter().map(|&i| values[i]));
                        f(&scratch)
                    }
                    Eval::Gaze(axis) => match axis {
                        GazeAxis::LeftX => gaze.left_x,
                        GazeAxis::LeftY => gaze.left_y,
                        GazeAxis::RightX => gaze.right_x,
                        GazeAxis::RightY => gaze.right_y,
                        GazeAxis::X => (gaze.left_x + gaze.right_x) * 0.5,
                        GazeAxis::Y => (gaze.left_y + gaze.right_y) * 0.5,
                    },
                    Eval::Zero => 0.0,
                };
                if output.signed { value.clamp(-1.0, 1.0) } else { value.clamp(0.0, 1.0) }
            })
            .collect()
    }

    /// Outputs for a neutral face, sent while FT output is switched off.
    pub fn neutral(&self) -> Vec<f32> {
        self.evaluate(&[0.0; COUNT], Gaze::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = OUTPUTS.iter().map(|o| o.name).collect();
        names.sort();
        let len = names.len();
        names.dedup();
        assert_eq!(len, names.len());
    }

    #[test]
    fn mapping_matches_reference_formulas() {
        let mapper = Mapper::new();
        let mut v = [0.0; COUNT];
        v[channels::index("eyeBlinkLeft")] = 0.4;
        v[channels::index("eyeWideRight")] = 0.8;
        v[channels::index("browInnerUp")] = 0.6;
        v[channels::index("browOuterUpLeft")] = 0.2;
        v[channels::index("browDownLeft")] = 0.1;
        let out = mapper.evaluate(&v, Gaze::default());
        let get = |n: &str| out[OUTPUTS.iter().position(|o| o.name == n).unwrap()];
        assert!((get("EyeLidLeft") - 0.45).abs() < 1e-6);
        assert!((get("EyeLidRight") - 0.95).abs() < 1e-6);
        assert!((get("BrowExpressionLeft") - 0.3).abs() < 1e-6);
        assert!((get("BrowUp") - 0.3).abs() < 1e-6);
        assert!((mapper.neutral()[OUTPUTS.iter().position(|o| o.name == "EyeLid").unwrap()] - 0.75).abs() < 1e-6);
    }
}
