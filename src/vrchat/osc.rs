//! OSC output to VRChat, including VRCFaceTracking-style binary parameter encoding.

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::Arc;
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
/// `#bundle\0` and the time tag.
const BUNDLE_HEADER: usize = 16;

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

/// Where every output goes, in [`OUTPUTS`] order; shared between threads without copying.
pub type Bindings = Arc<[Option<Binding>]>;

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

    /// The level `value` takes in the bits (bit `i` set means the `i`-th parameter is on),
    /// and its sign.
    pub fn encode(&self, value: f32) -> (u32, bool) {
        let steps = (1u32 << self.bit_count()) - 1;
        ((value.abs().min(1.0) * steps as f32).round() as u32, value < 0.0)
    }

    /// Every bool parameter with its state for `value`: the bits, low bit first, then the sign.
    pub fn states(&self, value: f32) -> impl Iterator<Item = (&str, bool)> {
        let (level, negative) = self.encode(value);
        let bits = self.bits.iter().enumerate().map(move |(i, address)| (address.as_str(), level >> i & 1 == 1));
        bits.chain(self.negative.as_deref().map(|address| (address, negative)))
    }
}

/// Name of a parameter after its `v2/` segment, e.g. `FT/v2/JawOpen` -> `JawOpen`.
pub fn v2_name(path: &str) -> Option<&str> {
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
    /// Host and port `target` was resolved from.
    target_key: (String, u16),
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
            target_key: (String::new(), 0),
            target: None,
            last: HashMap::new(),
            last_full_send: Instant::now(),
            pending: Vec::new(),
            pending_bytes: 0,
        })
    }

    pub fn set_target(&mut self, host: &str, port: u16) {
        if self.target_key.0 == host && self.target_key.1 == port {
            return;
        }
        self.target = (host, port).to_socket_addrs().ok().and_then(|mut a| a.next());
        self.target_key = (host.to_string(), port);
        self.last.clear();
    }

    /// Forget what was sent so the next frame re-sends everything.
    pub fn invalidate(&mut self) {
        self.last.clear();
    }

    fn queue(&mut self, address: &str, value: OscType, force: bool) -> Result<()> {
        match self.last.get_mut(address) {
            Some(last) if *last == value && !force => return Ok(()),
            Some(last) => *last = value.clone(),
            None => {
                self.last.insert(address.to_string(), value.clone());
            }
        }
        // Element size, then the address (with at least one NUL) and the type tag, each padded
        // to 4 bytes, then the argument: four bytes for a float, none for a bool.
        let argument = if matches!(value, OscType::Float(_)) { 4 } else { 0 };
        let size = 4 + (address.len() / 4 + 1) * 4 + 4 + argument;
        if self.pending_bytes + size > MAX_BUNDLE_BYTES {
            self.flush()?;
        }
        if self.pending.is_empty() {
            self.pending_bytes = BUNDLE_HEADER;
        }
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
        debug_assert!(bytes.len() <= MAX_BUNDLE_BYTES, "bundle of {} bytes", bytes.len());
        let (host, port) = &self.target_key;
        let addr = self.target.with_context(|| format!("cannot resolve OSC target {host}:{port}"))?;
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
            for (address, on) in binding.states(value) {
                self.queue(address, OscType::Bool(on), force)?;
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
    use crate::mapping::output_index;

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
        let get = |name: &str| bindings[output_index(name).unwrap()].clone();
        assert_eq!(get("JawOpen"), Some(Binding::float("/avatar/parameters/FT/v2/JawOpen".into())));
        let b = get("SmileFrown").unwrap();
        assert_eq!(b.float, None);
        assert_eq!(b.bit_count(), 3);
        assert!(b.negative.is_some());
        assert_eq!(b.encode(-1.0), (0b111, true));
        assert_eq!(b.encode(0.5), (0b100, false));
        let states: Vec<bool> = b.states(-1.0 / 7.0).map(|(_, on)| on).collect();
        assert_eq!(states, [true, false, false, true]);
        // Both a local float and synced bits: both are bound.
        let both = bind(&AvatarInfo {
            id: None,
            name: None,
            params: vec![param("FT/v2/JawOpen", ParamType::Float), param("FT/v2/JawOpen1", ParamType::Bool)],
        });
        let jaw = both[output_index("JawOpen").unwrap()].clone().unwrap();
        assert!(jaw.float.is_some() && jaw.bit_count() == 1);
        assert!(get("EyeX").is_none());
    }
}
