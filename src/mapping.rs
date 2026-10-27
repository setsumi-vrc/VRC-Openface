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

/// Position of the output called `name` in [`OUTPUTS`].
pub fn output_index(name: &str) -> Option<usize> {
    OUTPUTS.iter().position(|o| o.name == name)
}

impl Output {
    const fn new(name: &'static str, inputs: &'static [&'static str], eval: Eval) -> Self {
        Output { name, friendly: name, inputs, eval, signed: false, cat_ears: false }
    }

    /// A shorter label for "Friendly names" than the parameter name.
    const fn friendly(mut self, friendly: &'static str) -> Self {
        self.friendly = friendly;
        self
    }

    const fn signed(mut self) -> Self {
        self.signed = true;
        self
    }
}

const fn out(name: &'static str, inputs: &'static [&'static str], f: fn(&[f32]) -> f32) -> Output {
    Output::new(name, inputs, Eval::F(f))
}

const fn signed(name: &'static str, inputs: &'static [&'static str], f: fn(&[f32]) -> f32) -> Output {
    out(name, inputs, f).signed()
}

const fn direct(name: &'static str, input: &'static [&'static str]) -> Output {
    out(name, input, first)
}

const fn mean_of(name: &'static str, inputs: &'static [&'static str]) -> Output {
    out(name, inputs, mean)
}

const fn gaze(name: &'static str, inputs: &'static [&'static str], axis: GazeAxis) -> Output {
    Output::new(name, inputs, Eval::Gaze(axis)).signed()
}

const fn zero(name: &'static str) -> Output {
    Output::new(name, &[], Eval::Zero)
}

const fn ear(name: &'static str, inputs: &'static [&'static str]) -> Output {
    let mut output = signed(name, inputs, ear_pose);
    output.cat_ears = true;
    output
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
    out("EyeLidRight", &["eyeBlinkRight", "eyeWideRight"], |v| lid(v[0], v[1])),
    out("EyeLidLeft", &["eyeBlinkLeft", "eyeWideLeft"], |v| lid(v[0], v[1])),
    out("EyeLid", &["eyeBlinkLeft", "eyeWideLeft", "eyeBlinkRight", "eyeWideRight"], |v| {
        (lid(v[0], v[1]) + lid(v[2], v[3])) * 0.5
    }),
    direct("EyeSquintRight", &["eyeSquintRight"]),
    direct("EyeSquintLeft", &["eyeSquintLeft"]),
    mean_of("EyeSquint", &["eyeSquintLeft", "eyeSquintRight"]),
    out("EyesSquint", &["eyeSquintLeft", "eyeSquintRight"], max2),
    out("EyeOpenLeft", &["eyeBlinkLeft"], open),
    out("EyeOpenRight", &["eyeBlinkRight"], open),
    out("EyeOpen", &["eyeBlinkLeft", "eyeBlinkRight"], open),
    direct("EyeClosedLeft", &["eyeBlinkLeft"]),
    direct("EyeClosedRight", &["eyeBlinkRight"]),
    mean_of("EyeClosed", &["eyeBlinkLeft", "eyeBlinkRight"]),
    direct("EyeWideLeft", &["eyeWideLeft"]),
    direct("EyeWideRight", &["eyeWideRight"]),
    out("EyeWide", &["eyeWideLeft", "eyeWideRight"], max2),
    // Brows
    direct("BrowLowererLeft", &["browDownLeft"]),
    direct("BrowLowererRight", &["browDownRight"]),
    direct("BrowInnerUpLeft", &["browInnerUp"]),
    direct("BrowInnerUpRight", &["browInnerUp"]),
    direct("BrowOuterUpLeft", &["browOuterUpLeft"]),
    direct("BrowOuterUpRight", &["browOuterUpRight"]),
    out("BrowUp", &["browInnerUp", "browOuterUpLeft", "browOuterUpRight"], brow_up),
    signed("BrowExpressionRight", &["browInnerUp", "browOuterUpRight", "browDownRight"], |v| {
        brow_expression(v[0], v[1], v[2])
    })
    .friendly("BrowExprRight"),
    signed("BrowExpressionLeft", &["browInnerUp", "browOuterUpLeft", "browDownLeft"], |v| {
        brow_expression(v[0], v[1], v[2])
    })
    .friendly("BrowExprLeft"),
    signed(
        "BrowExpression",
        &["browInnerUp", "browOuterUpLeft", "browDownLeft", "browOuterUpRight", "browDownRight"],
        |v| (brow_expression(v[0], v[1], v[2]) + brow_expression(v[0], v[3], v[4])) * 0.5,
    ),
    direct("BrowDownLeft", &["browDownLeft"]),
    direct("BrowDownRight", &["browDownRight"]),
    mean_of("BrowDown", &["browDownLeft", "browDownRight"]),
    direct("BrowInnerUp", &["browInnerUp"]),
    mean_of("BrowOuterUp", &["browOuterUpLeft", "browOuterUpRight"]),
    // Jaw, nose, cheeks
    direct("JawOpen", &["jawOpen"]),
    direct("MouthClosed", &["mouthClose"]),
    signed("JawX", &["jawRight", "jawLeft"], diff),
    direct("JawZ", &["jawForward"]),
    direct("NoseSneerLeft", &["noseSneerLeft"]),
    direct("NoseSneerRight", &["noseSneerRight"]),
    mean_of("NoseSneer", &["noseSneerLeft", "noseSneerRight"]),
    direct("CheekSquintLeft", &["cheekSquintLeft"]),
    direct("CheekSquintRight", &["cheekSquintRight"]),
    mean_of("CheekSquint", &["cheekSquintLeft", "cheekSquintRight"]),
    direct("CheekPuffSuckLeft", &["cheekPuff"]).friendly("CheekPuffLeft"),
    direct("CheekPuffSuckRight", &["cheekPuff"]).friendly("CheekPuffRight"),
    direct("CheekPuffSuck", &["cheekPuff"]).friendly("CheekPuff"),
    // Mouth
    direct("MouthUpperUpLeft", &["mouthUpperUpLeft"]),
    direct("MouthUpperUpRight", &["mouthUpperUpRight"]),
    direct("MouthLowerDownLeft", &["mouthLowerDownLeft"]),
    direct("MouthLowerDownRight", &["mouthLowerDownRight"]),
    direct("MouthCornerPullLeft", &["mouthSmileLeft"]).friendly("CornerPullLeft"),
    direct("MouthCornerPullRight", &["mouthSmileRight"]).friendly("CornerPullRight"),
    direct("MouthFrownLeft", &["mouthFrownLeft"]),
    direct("MouthFrownRight", &["mouthFrownRight"]),
    direct("MouthStretchLeft", &["mouthStretchLeft"]).friendly("StretchLeft"),
    direct("MouthStretchRight", &["mouthStretchRight"]).friendly("StretchRight"),
    mean_of("MouthStretch", &["mouthStretchLeft", "mouthStretchRight"]).friendly("Stretch"),
    direct("MouthDimpleLeft", &["mouthDimpleLeft"]).friendly("DimpleLeft"),
    direct("MouthDimpleRight", &["mouthDimpleRight"]).friendly("DimpleRight"),
    mean_of("MouthDimple", &["mouthDimpleLeft", "mouthDimpleRight"]).friendly("Dimple"),
    direct("MouthPressLeft", &["mouthPressLeft"]).friendly("PressLeft"),
    direct("MouthPressRight", &["mouthPressRight"]).friendly("PressRight"),
    mean_of("MouthPress", &["mouthPressLeft", "mouthPressRight"]).friendly("Press"),
    direct("MouthRaiserUpper", &["mouthShrugUpper"]).friendly("RaiserUpper"),
    direct("MouthRaiserLower", &["mouthShrugLower"]).friendly("RaiserLower"),
    // Lips
    direct("LipFunnelUpperRight", &["mouthFunnel"]).friendly("FunnelUpR"),
    direct("LipFunnelUpperLeft", &["mouthFunnel"]).friendly("FunnelUpL"),
    direct("LipFunnelLowerRight", &["mouthFunnel"]).friendly("FunnelLoR"),
    direct("LipFunnelLowerLeft", &["mouthFunnel"]).friendly("FunnelLoL"),
    direct("LipFunnelUpper", &["mouthFunnel"]).friendly("FunnelUpper"),
    direct("LipFunnelLower", &["mouthFunnel"]).friendly("FunnelLower"),
    direct("LipFunnel", &["mouthFunnel"]),
    direct("LipPuckerUpperRight", &["mouthPucker"]).friendly("PuckerUpR"),
    direct("LipPuckerUpperLeft", &["mouthPucker"]).friendly("PuckerUpL"),
    direct("LipPuckerLowerRight", &["mouthPucker"]).friendly("PuckerLoR"),
    direct("LipPuckerLowerLeft", &["mouthPucker"]).friendly("PuckerLoL"),
    direct("LipPuckerUpper", &["mouthPucker"]).friendly("PuckerUpper"),
    direct("LipPuckerLower", &["mouthPucker"]).friendly("PuckerLower"),
    direct("LipPucker", &["mouthPucker"]),
    direct("LipPuckerLeft", &["mouthPucker"]).friendly("PuckerLeft"),
    direct("LipPuckerRight", &["mouthPucker"]).friendly("PuckerRight"),
    direct("LipSuckUpperLeft", &["mouthRollUpper"]).friendly("SuckUpL"),
    direct("LipSuckUpperRight", &["mouthRollUpper"]).friendly("SuckUpR"),
    direct("LipSuckLowerLeft", &["mouthRollLower"]).friendly("SuckLoL"),
    direct("LipSuckLowerRight", &["mouthRollLower"]).friendly("SuckLoR"),
    direct("LipSuckUpper", &["mouthRollUpper"]).friendly("SuckUpper"),
    direct("LipSuckLower", &["mouthRollLower"]).friendly("SuckLower"),
    mean_of("LipSuck", &["mouthRollUpper", "mouthRollLower"]),
    signed("MouthX", &["mouthRight", "mouthLeft"], diff),
    signed("MouthUpperX", &["mouthRight", "mouthLeft"], diff),
    signed("MouthLowerX", &["mouthRight", "mouthLeft"], diff),
    mean_of("MouthOpen", &["mouthUpperUpLeft", "mouthUpperUpRight", "mouthLowerDownLeft", "mouthLowerDownRight"]),
    direct("MouthSmileLeft", &["mouthSmileLeft"]).friendly("SmileLeft"),
    direct("MouthSmileRight", &["mouthSmileRight"]).friendly("SmileRight"),
    out("MouthSadLeft", &["mouthFrownLeft", "mouthStretchLeft"], max2),
    out("MouthSadRight", &["mouthFrownRight", "mouthStretchRight"], max2),
    mean_of("MouthUpperUp", &["mouthUpperUpLeft", "mouthUpperUpRight"]).friendly("UpperUp"),
    mean_of("MouthLowerDown", &["mouthLowerDownLeft", "mouthLowerDownRight"]).friendly("LowerDown"),
    signed("SmileFrownRight", &["mouthSmileRight", "mouthFrownRight"], diff).friendly("SmileFrownR"),
    signed("SmileFrownLeft", &["mouthSmileLeft", "mouthFrownLeft"], diff).friendly("SmileFrownL"),
    signed("SmileFrown", &["mouthSmileRight", "mouthFrownRight", "mouthSmileLeft", "mouthFrownLeft"], |v| {
        ((v[0] - v[1]) + (v[2] - v[3])) * 0.5
    }),
    signed("SmileSadRight", &["mouthSmileRight", "mouthFrownRight", "mouthStretchRight"], |v| v[0] - v[1].max(v[2]))
        .friendly("SmileSadR"),
    signed("SmileSadLeft", &["mouthSmileLeft", "mouthFrownLeft", "mouthStretchLeft"], |v| v[0] - v[1].max(v[2]))
        .friendly("SmileSadL"),
    signed(
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
    direct("TongueOut", &["tongueOut"]),
    signed("TongueX", &["tongueRight", "tongueLeft"], diff),
    signed("TongueY", &["tongueUp", "tongueDown"], diff),
    direct("TongueRoll", &["tongueRoll"]),
    zero("TongueArchY"),
    zero("TongueShape"),
    zero("TongueTwistRight"),
    zero("TongueTwistLeft"),
    // Cat ears (our own addition)
    ear("EarLeft", &["browInnerUp", "browOuterUpLeft", "browDownLeft", "eyeWideLeft"]),
    ear("EarRight", &["browInnerUp", "browOuterUpRight", "browDownRight", "eyeWideRight"]),
];

/// The most inputs one formula reads.
const MAX_INPUTS: usize = 8;

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
        let resolve = |o: &Output| -> Vec<usize> {
            assert!(o.inputs.len() <= MAX_INPUTS, "{} reads too many channels", o.name);
            o.inputs.iter().map(|n| channels::find(n).unwrap_or_else(|| panic!("unknown channel {n}"))).collect()
        };
        Self { indices: OUTPUTS.iter().map(resolve).collect() }
    }

    pub fn dependencies(&self, output: usize) -> &[usize] {
        &self.indices[output]
    }

    pub fn evaluate(&self, values: &[f32; COUNT], gaze: Gaze) -> Vec<f32> {
        let mut scratch = [0.0; MAX_INPUTS];
        OUTPUTS
            .iter()
            .zip(&self.indices)
            .map(|(output, indices)| {
                let value = match output.eval {
                    Eval::F(f) => {
                        for (s, &i) in scratch.iter_mut().zip(indices) {
                            *s = values[i];
                        }
                        f(&scratch[..indices.len()])
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
        for (name, value) in [
            ("eyeBlinkLeft", 0.4),
            ("eyeWideRight", 0.8),
            ("browInnerUp", 0.6),
            ("browOuterUpLeft", 0.2),
            ("browDownLeft", 0.1),
        ] {
            v[channels::index(name)] = value;
        }
        let out = mapper.evaluate(&v, Gaze::default());
        let get = |n: &str| out[output_index(n).unwrap()];
        assert_eq!(
            OUTPUTS.iter().find(|o| o.name == "MouthCornerPullLeft").map(|o| o.friendly),
            Some("CornerPullLeft")
        );
        assert_eq!(OUTPUTS.iter().find(|o| o.name == "JawOpen").map(|o| o.label(true)), Some("JawOpen"));
        assert!((get("EyeLidLeft") - 0.45).abs() < 1e-6);
        assert!((get("EyeLidRight") - 0.95).abs() < 1e-6);
        assert!((get("BrowExpressionLeft") - 0.3).abs() < 1e-6);
        assert!((get("BrowUp") - 0.3).abs() < 1e-6);
        assert!((mapper.neutral()[output_index("EyeLid").unwrap()] - 0.75).abs() < 1e-6);
    }
}
