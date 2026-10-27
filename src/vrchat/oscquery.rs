//! Avatar parameter discovery through VRChat's OSCQuery service.
//!
//! VRChat advertises `_oscjson._tcp` over mDNS. Its HTTP endpoint serves the avatar
//! parameter tree, which tells us which face tracking parameters the current avatar
//! has and whether they are floats or binary-encoded bools.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use parking_lot::Mutex;
use serde_json::Value;

use crate::vrchat::osc::{AvatarInfo, AvatarParam, ParamType};

const SERVICE: &str = "_oscjson._tcp.local.";
const POLL: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct DiscoveryState {
    pub endpoint: Option<SocketAddr>,
    pub avatar: Option<AvatarInfo>,
    pub error: Option<String>,
    /// Bumped whenever `avatar` changes so consumers can rebind cheaply.
    pub generation: u64,
}

pub type SharedDiscovery = Arc<Mutex<DiscoveryState>>;

pub fn spawn() -> SharedDiscovery {
    let state = SharedDiscovery::default();
    let shared = state.clone();
    thread::Builder::new()
        .name("oscquery".into())
        .spawn(move || {
            if let Err(e) = run(&shared) {
                log::warn!("OSCQuery discovery stopped: {e:#}");
                shared.lock().error = Some(format!("{e:#}"));
            }
        })
        .expect("spawning OSCQuery thread");
    state
}

fn run(state: &SharedDiscovery) -> Result<()> {
    let mdns = ServiceDaemon::new().context("starting mDNS")?;
    let events = mdns.browse(SERVICE).context("browsing for OSCQuery services")?;
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(2))).build().into();
    let mut last_poll: Option<Instant> = None;
    let mut last_raw = String::new();

    loop {
        loop {
            let event = match events.recv_timeout(Duration::from_millis(250)) {
                Ok(event) => event,
                Err(e) if events.is_disconnected() => bail!("mDNS stopped: {e}"),
                Err(_) => break,
            };
            match event {
                ServiceEvent::ServiceResolved(info) if info.get_fullname().starts_with("VRChat-Client-") => {
                    let addresses = info.get_addresses_v4();
                    let ip = addresses.iter().find(|a| a.is_loopback()).or(addresses.iter().next()).copied();
                    if let Some(ip) = ip {
                        let endpoint = SocketAddr::new(IpAddr::V4(ip), info.get_port());
                        log::info!("found VRChat OSCQuery at {endpoint}");
                        state.lock().endpoint = Some(endpoint);
                    }
                }
                ServiceEvent::ServiceRemoved(_, name) if name.starts_with("VRChat-Client-") => {
                    let mut s = state.lock();
                    s.endpoint = None;
                    s.avatar = None;
                    s.generation += 1;
                    last_raw.clear();
                }
                _ => {}
            }
        }

        if last_poll.is_some_and(|at| at.elapsed() < POLL) {
            continue;
        }
        last_poll = Some(Instant::now());
        let Some(endpoint) = state.lock().endpoint else { continue };
        match fetch(&agent, endpoint) {
            Ok(raw) if raw == last_raw => state.lock().error = None,
            Ok(raw) => {
                let avatar = parse_avatar(&raw);
                let mut s = state.lock();
                s.error = None;
                s.generation += 1;
                s.avatar = avatar.ok();
                last_raw = raw;
            }
            Err(e) => {
                let mut s = state.lock();
                s.error = Some(format!("{e:#}"));
                if s.avatar.take().is_some() {
                    s.generation += 1;
                }
                last_raw.clear();
            }
        }
    }
}

fn fetch(agent: &ureq::Agent, endpoint: SocketAddr) -> Result<String> {
    let url = format!("http://{endpoint}/avatar");
    Ok(agent.get(&url).call()?.body_mut().read_to_string()?)
}

pub fn parse_avatar(raw: &str) -> Result<AvatarInfo> {
    let root: Value = serde_json::from_str(raw)?;
    let mut info = AvatarInfo::default();
    collect(&root, &mut info);
    info.params.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(info)
}

fn collect(node: &Value, info: &mut AvatarInfo) {
    let path = node.get("FULL_PATH").and_then(Value::as_str).unwrap_or_default();
    if path == "/avatar/change" {
        info.id = node.get("VALUE").and_then(|v| v.get(0)).and_then(Value::as_str).map(str::to_string);
    } else if let (Some(name), Some(kind)) =
        (path.strip_prefix("/avatar/parameters/"), node.get("TYPE").and_then(Value::as_str))
    {
        let kind = match kind {
            "f" => Some(ParamType::Float),
            "i" => Some(ParamType::Int),
            "T" | "F" => Some(ParamType::Bool),
            _ => None,
        };
        if let Some(kind) = kind {
            info.params.push(AvatarParam { path: name.to_string(), kind });
        }
    }
    if let Some(children) = node.get("CONTENTS").and_then(Value::as_object) {
        for child in children.values() {
            collect(child, info);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vrchat_avatar_tree() {
        let raw = r#"{"FULL_PATH":"/avatar","CONTENTS":{
            "change":{"FULL_PATH":"/avatar/change","TYPE":"s","VALUE":["avtr_123"]},
            "parameters":{"FULL_PATH":"/avatar/parameters","CONTENTS":{
                "FT":{"FULL_PATH":"/avatar/parameters/FT","CONTENTS":{
                    "v2":{"FULL_PATH":"/avatar/parameters/FT/v2","CONTENTS":{
                        "JawOpen":{"FULL_PATH":"/avatar/parameters/FT/v2/JawOpen","TYPE":"f","VALUE":[0.0]},
                        "EyeLid1":{"FULL_PATH":"/avatar/parameters/FT/v2/EyeLid1","TYPE":"T","VALUE":[false]}
                    }}}},
                "VRCEmote":{"FULL_PATH":"/avatar/parameters/VRCEmote","TYPE":"i","VALUE":[0]}
            }}}}"#;
        let info = parse_avatar(raw).unwrap();
        assert_eq!(info.id.as_deref(), Some("avtr_123"));
        assert_eq!(info.params.len(), 3);
        assert!(info.params.iter().any(|p| p.path == "FT/v2/EyeLid1" && p.kind == ParamType::Bool));
    }
}
