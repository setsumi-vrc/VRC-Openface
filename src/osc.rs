//! OSC output to VRChat, including VRCFaceTracking-style binary parameter encoding.

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rosc::{OscMessage, OscPacket, OscType};

use crate::unified::PARAMS;

pub const PARAMETER_ROOT: &str = "/avatar/parameters/";
/// Prefix used when the avatar's parameter list is unknown.
pub const DEFAULT_PREFIX: &str = "FT/v2/";
const BINARY_STEPS: [u32; 8] = [1, 2, 4, 8, 16, 32, 64, 128];
const TRACKING_FLAGS: [&str; 2] = ["EyeTrackingActive", "LipTrackingActive"];

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
    pub params: Vec<AvatarParam>,
}

#[derive(Clone, Debug)]
pub enum Binding {
    Float(String),
    /// Bool parameters for 1, 2, 4, ... steps, plus an optional sign bit.
    Binary {
        bits: Vec<String>,
        negative: Option<String>,
    },
}

impl Binding {
    pub fn describe(&self) -> String {
        match self {
            Binding::Float(address) => address.trim_start_matches(PARAMETER_ROOT).to_string(),
            Binding::Binary { bits, negative } => {
                let first = bits.first().map(|b| b.trim_start_matches(PARAMETER_ROOT)).unwrap_or_default();
                let sign = if negative.is_some() { " ±" } else { "" };
                format!("{} ({} bit{sign})", first.trim_end_matches('1'), bits.len())
            }
        }
    }
}

/// Name of a parameter after its `v2/` segment, e.g. `FT/v2/JawOpen` -> `JawOpen`.
fn v2_name(path: &str) -> Option<&str> {
    let at = path.rfind("v2/")?;
    (at == 0 || path.as_bytes()[at - 1] == b'/').then(|| &path[at + 3..])
}

/// Matches every Unified Expressions parameter against the avatar's parameter list.
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
    PARAMS
        .iter()
        .map(|param| {
            if let Some(path) = floats.get(param.name) {
                return Some(Binding::Float(format!("{PARAMETER_ROOT}{path}")));
            }
            let bits: Vec<String> = BINARY_STEPS
                .iter()
                .map_while(|step| bools.get(format!("{}{step}", param.name).as_str()))
                .map(|path| format!("{PARAMETER_ROOT}{path}"))
                .collect();
            if bits.is_empty() {
                return None;
            }
            let negative =
                bools.get(format!("{}Negative", param.name).as_str()).map(|path| format!("{PARAMETER_ROOT}{path}"));
            Some(Binding::Binary { bits, negative })
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
    PARAMS.iter().map(|p| Some(Binding::Float(format!("{PARAMETER_ROOT}{DEFAULT_PREFIX}{}", p.name)))).collect()
}

pub struct OscSender {
    socket: UdpSocket,
    target_key: String,
    target: Option<SocketAddr>,
    last: HashMap<String, OscType>,
    last_full_send: Instant,
}

impl OscSender {
    pub fn new() -> Result<Self> {
        Ok(Self {
            socket: UdpSocket::bind(("0.0.0.0", 0)).context("binding OSC socket")?,
            target_key: String::new(),
            target: None,
            last: HashMap::new(),
            last_full_send: Instant::now(),
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

    fn send(&mut self, address: &str, value: OscType, force: bool) -> Result<()> {
        if !force && self.last.get(address) == Some(&value) {
            return Ok(());
        }
        let packet = OscPacket::Message(OscMessage { addr: address.to_string(), args: vec![value.clone()] });
        let bytes = rosc::encoder::encode(&packet)?;
        let addr = self.target.with_context(|| format!("cannot resolve OSC target {}", self.target_key))?;
        self.socket.send_to(&bytes, addr)?;
        self.last.insert(address.to_string(), value);
        Ok(())
    }

    /// Sends changed values; everything is re-sent once a second so a late-joining
    /// avatar still receives the full state.
    pub fn send_frame(&mut self, bindings: &[Option<Binding>], values: &[f32], flags: &[String]) -> Result<()> {
        let force = self.last_full_send.elapsed() >= Duration::from_secs(1);
        if force {
            self.last_full_send = Instant::now();
        }
        for (binding, &value) in bindings.iter().zip(values) {
            match binding {
                None => {}
                Some(Binding::Float(address)) => {
                    // Quantise so sub-visible jitter does not flood the network.
                    let value = (value * 1000.0).round() / 1000.0;
                    self.send(address, OscType::Float(value), force)?;
                }
                Some(Binding::Binary { bits, negative }) => {
                    let steps = (1u32 << bits.len()) - 1;
                    let level = (value.abs().min(1.0) * steps as f32).round() as u32;
                    for (i, address) in bits.iter().enumerate() {
                        self.send(address, OscType::Bool(level & (1 << i) != 0), force)?;
                    }
                    if let Some(address) = negative {
                        self.send(address, OscType::Bool(value < 0.0), force)?;
                    }
                }
            }
        }
        for flag in flags {
            self.send(flag, OscType::Bool(true), force)?;
        }
        Ok(())
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
        let get = |name: &str| bindings[PARAMS.iter().position(|p| p.name == name).unwrap()].clone();
        assert!(matches!(get("JawOpen"), Some(Binding::Float(a)) if a == "/avatar/parameters/FT/v2/JawOpen"));
        match get("SmileFrown") {
            Some(Binding::Binary { bits, negative }) => {
                assert_eq!(bits.len(), 3);
                assert!(negative.is_some());
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(get("EyeX").is_none());
    }
}
