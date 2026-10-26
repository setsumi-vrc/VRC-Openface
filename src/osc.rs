//! OSC output to VRChat, including VRCFaceTracking-style binary parameter encoding.

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rosc::{OscBundle, OscMessage, OscPacket, OscTime, OscType};

use crate::mapping::OUTPUTS;

pub const PARAMETER_ROOT: &str = "/avatar/parameters/";
/// Prefix used when the avatar's parameter list is unknown.
pub const DEFAULT_PREFIX: &str = "FT/v2/";
const BINARY_STEPS: [u32; 8] = [1, 2, 4, 8, 16, 32, 64, 128];
const TRACKING_FLAGS: [&str; 2] = ["EyeTrackingActive", "LipTrackingActive"];
/// Keeps each datagram well below the loopback MTU VRChat reads in one go.
const MAX_BUNDLE_BYTES: usize = 4000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamType {
    Float,
    Int,
    Bool,
}

#[derive(Clone, Debug)]
pub struct AvatarParam {
    /// Path below `/avatar/parameters/`, e.g. `FT/v2/JawOpen`.
    pub path: String,
    pub kind: ParamType,
}

#[derive(Clone, Debug, Default)]
pub struct AvatarInfo {
    pub id: Option<String>,
    pub name: Option<String>,
    pub params: Vec<AvatarParam>,
}

/// Where one output goes on the avatar. Avatars often declare both a local float and
/// synced binary bools for the same expression; both are sent, like VRCFaceTracking.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Binding {
    pub float: Option<String>,
    /// Bool parameters for 1, 2, 4, ... steps.
    pub bits: Vec<String>,
    pub negative: Option<String>,
}

impl Binding {
    pub fn float(address: String) -> Self {
        Self { float: Some(address), ..Default::default() }
    }

    pub fn bit_count(&self) -> usize {
        self.bits.len()
    }

    /// Bits that are set for `value`, low bit first, plus the sign.
    pub fn encode(&self, value: f32) -> (Vec<bool>, bool) {
        let n = self.bit_count();
        if n == 0 {
            return (Vec::new(), value < 0.0);
        }
        let steps = (1u32 << n) - 1;
        let level = (value.abs().min(1.0) * steps as f32).round() as u32;
        ((0..n).map(|i| level & (1 << i) != 0).collect(), value < 0.0)
    }
}

/// Name of a parameter after its `v2/` segment, e.g. `FT/v2/JawOpen` -> `JawOpen`.
fn v2_name(path: &str) -> Option<&str> {
    let at = path.rfind("v2/")?;
    (at == 0 || path.as_bytes()[at - 1] == b'/').then(|| &path[at + 3..])
}

/// Matches every output against the avatar's parameter list.
pub fn bind(avatar: &AvatarInfo) -> Vec<Option<Binding>> {
    let mut floats: HashMap<&str, &str> = HashMap::new();
    let mut bools: HashMap<&str, &str> = HashMap::new();
    for p in &avatar.params {
        let Some(name) = v2_name(&p.path) else { continue };
        match p.kind {
            ParamType::Float | ParamType::Int => floats.insert(name, &p.path),
            ParamType::Bool => bools.insert(name, &p.path),
        };
    }
    OUTPUTS
        .iter()
        .map(|output| {
            let float = floats.get(output.name).map(|path| format!("{PARAMETER_ROOT}{path}"));
            let bits: Vec<String> = BINARY_STEPS
                .iter()
                .map_while(|step| bools.get(format!("{}{step}", output.name).as_str()))
                .map(|path| format!("{PARAMETER_ROOT}{path}"))
                .collect();
            let negative = (!bits.is_empty())
                .then(|| bools.get(format!("{}Negative", output.name).as_str()))
                .flatten()
                .map(|path| format!("{PARAMETER_ROOT}{path}"));
            (float.is_some() || !bits.is_empty()).then_some(Binding { float, bits, negative })
        })
        .collect()
}

pub fn tracking_flags(avatar: &AvatarInfo) -> Vec<String> {
    avatar
        .params
        .iter()
        .filter(|p| p.kind == ParamType::Bool && v2_name(&p.path).is_some_and(|n| TRACKING_FLAGS.contains(&n)))
        .map(|p| format!("{PARAMETER_ROOT}{}", p.path))
        .collect()
}

pub fn default_bindings() -> Vec<Option<Binding>> {
    OUTPUTS.iter().map(|o| Some(Binding::float(format!("{PARAMETER_ROOT}{DEFAULT_PREFIX}{}", o.name)))).collect()
}

pub struct OscSender {
    socket: UdpSocket,
    target_key: String,
    target: Option<SocketAddr>,
    last: HashMap<String, OscType>,
    last_full_send: Instant,
    pending: Vec<OscPacket>,
    pending_bytes: usize,
}

impl OscSender {
    pub fn new() -> Result<Self> {
        Ok(Self {
            socket: UdpSocket::bind(("0.0.0.0", 0)).context("binding OSC socket")?,
            target_key: String::new(),
            target: None,
            last: HashMap::new(),
            last_full_send: Instant::now(),
            pending: Vec::new(),
            pending_bytes: 0,
        })
    }

    pub fn set_target(&mut self, host: &str, port: u16) {
        let key = format!("{host}:{port}");
        if key != self.target_key {
            self.target = key.to_socket_addrs().ok().and_then(|mut a| a.next());
            self.target_key = key;
            self.last.clear();
        }
    }

    /// Forget what was sent so the next frame re-sends everything.
    pub fn invalidate(&mut self) {
        self.last.clear();
    }

    fn queue(&mut self, address: &str, value: OscType, force: bool) -> Result<()> {
        if !force && self.last.get(address) == Some(&value) {
            return Ok(());
        }
        // Address, type tag and argument, each padded to 4 bytes.
        let size = (address.len() + 4) / 4 * 4 + 4 + 4 + 16;
        if self.pending_bytes + size > MAX_BUNDLE_BYTES {
            self.flush()?;
        }
        self.last.insert(address.to_string(), value.clone());
        self.pending.push(OscPacket::Message(OscMessage { addr: address.to_string(), args: vec![value] }));
        self.pending_bytes += size;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let content = std::mem::take(&mut self.pending);
        self.pending_bytes = 0;
        let packet = OscPacket::Bundle(OscBundle { timetag: OscTime { seconds: 0, fractional: 1 }, content });
        let bytes = rosc::encoder::encode(&packet)?;
        let addr = self.target.with_context(|| format!("cannot resolve OSC target {}", self.target_key))?;
        self.socket.send_to(&bytes, addr)?;
        Ok(())
    }

    /// Sends changed values as bundles, so every bit of a binary parameter lands in the
    /// same VRChat frame. Everything is re-sent once a second for late joiners.
    pub fn send_frame(
        &mut self,
        bindings: &[Option<Binding>],
        values: &[f32],
        flags: &[String],
        active: bool,
    ) -> Result<()> {
        let force = self.last_full_send.elapsed() >= Duration::from_secs(1);
        if force {
            self.last_full_send = Instant::now();
        }
        for (binding, &value) in bindings.iter().zip(values) {
            let Some(binding) = binding else { continue };
            if let Some(address) = &binding.float {
                // Quantise so sub-visible jitter does not flood the network.
                let value = (value * 1000.0).round() / 1000.0;
                self.queue(address, OscType::Float(value), force)?;
            }
            if !binding.bits.is_empty() {
                let (set, sign) = binding.encode(value);
                for (address, on) in binding.bits.iter().zip(set) {
                    self.queue(address, OscType::Bool(on), force)?;
                }
                if let Some(address) = &binding.negative {
                    self.queue(address, OscType::Bool(sign), force)?;
                }
            }
        }
        for flag in flags {
            self.queue(flag, OscType::Bool(active), force)?;
        }
        self.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(path: &str, kind: ParamType) -> AvatarParam {
        AvatarParam { path: path.into(), kind }
    }

    #[test]
    fn binds_float_and_binary_parameters() {
        let avatar = AvatarInfo {
            id: None,
            name: None,
            params: vec![
                param("FT/v2/JawOpen", ParamType::Float),
                param("FT/v2/SmileFrown1", ParamType::Bool),
                param("FT/v2/SmileFrown2", ParamType::Bool),
                param("FT/v2/SmileFrown4", ParamType::Bool),
                param("FT/v2/SmileFrownNegative", ParamType::Bool),
                param("JawOpen", ParamType::Float),
                param("notv2/EyeX", ParamType::Float),
            ],
        };
        let bindings = bind(&avatar);
        let get = |name: &str| bindings[OUTPUTS.iter().position(|p| p.name == name).unwrap()].clone();
        assert_eq!(get("JawOpen"), Some(Binding::float("/avatar/parameters/FT/v2/JawOpen".into())));
        let b = get("SmileFrown").unwrap();
        assert_eq!(b.float, None);
        assert_eq!(b.bit_count(), 3);
        assert!(b.negative.is_some());
        assert_eq!(b.encode(-1.0), (vec![true, true, true], true));
        assert_eq!(b.encode(0.5), (vec![false, false, true], false));
        // Both a local float and synced bits: both are bound.
        let both = bind(&AvatarInfo {
            id: None,
            name: None,
            params: vec![param("FT/v2/JawOpen", ParamType::Float), param("FT/v2/JawOpen1", ParamType::Bool)],
        });
        let jaw = both[OUTPUTS.iter().position(|p| p.name == "JawOpen").unwrap()].clone().unwrap();
        assert!(jaw.float.is_some() && jaw.bit_count() == 1);
        assert!(get("EyeX").is_none());
    }
}
