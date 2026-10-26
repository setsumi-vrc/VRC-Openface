//! Avatar information without OSCQuery: VRChat writes an OSC config JSON for every
//! avatar (name and parameters), and announces avatar switches with `/avatar/change`
//! on the receive port.

use std::fs;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};

use parking_lot::Mutex;
use rosc::{OscPacket, OscType};
use serde_json::Value;

use crate::osc::{AvatarInfo, AvatarParam, ParamType};

/// `%USERPROFILE%\AppData\LocalLow\VRChat\VRChat\OSC`.
fn osc_root() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join("AppData").join("LocalLow").join("VRChat").join("VRChat").join("OSC"))
}

/// Reads the newest config VRChat wrote for `avatar_id` across all local users.
pub fn load_config(avatar_id: &str) -> Option<AvatarInfo> {
    if !avatar_id.starts_with("avtr_") || avatar_id.contains(['/', '\\', '.']) {
        return None;
    }
    let root = osc_root()?;
    let newest = fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|user| user.path().join("Avatars").join(format!("{avatar_id}.json")))
        .filter_map(|path| Some((fs::metadata(&path).ok()?.modified().unwrap_or(SystemTime::UNIX_EPOCH), path)))
        .max()?
        .1;
    let text = fs::read_to_string(newest).ok()?;
    parse_config(text.trim_start_matches('\u{feff}'))
}

pub fn parse_config(text: &str) -> Option<AvatarInfo> {
    let root: Value = serde_json::from_str(text).ok()?;
    let params = root
        .get("parameters")?
        .as_array()?
        .iter()
        .filter_map(|p| {
            let path = p.get("name")?.as_str()?.to_string();
            let io = p.get("input").or_else(|| p.get("output"))?;
            let kind = match io.get("type")?.as_str()? {
                "Float" => ParamType::Float,
                "Int" => ParamType::Int,
                "Bool" => ParamType::Bool,
                _ => return None,
            };
            Some(AvatarParam { path, kind })
        })
        .collect();
    Some(AvatarInfo {
        id: root.get("id").and_then(Value::as_str).map(str::to_string),
        name: root.get("name").and_then(Value::as_str).map(str::to_string),
        params,
    })
}

#[derive(Default)]
pub struct ReceiverState {
    pub avatar_id: Option<String>,
    pub bound_port: Option<u16>,
    pub error: Option<String>,
    pub generation: u64,
}

pub type SharedReceiver = Arc<Mutex<ReceiverState>>;

/// Listens on the receive port for `/avatar/change`. Rebinds when `port` changes.
pub fn spawn_receiver(port: Arc<AtomicU16>, initial_avatar: Option<String>) -> SharedReceiver {
    let state = Arc::new(Mutex::new(ReceiverState { avatar_id: initial_avatar, ..Default::default() }));
    let shared = state.clone();
    thread::Builder::new()
        .name("osc-receive".into())
        .spawn(move || {
            let mut socket: Option<(u16, UdpSocket)> = None;
            let mut buffer = vec![0u8; 65536];
            loop {
                let wanted = port.load(Ordering::Relaxed);
                if socket.as_ref().map(|(p, _)| *p) != Some(wanted) {
                    socket = None;
                    match UdpSocket::bind(("127.0.0.1", wanted)) {
                        Ok(s) => {
                            let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
                            let mut st = shared.lock();
                            st.bound_port = Some(wanted);
                            st.error = None;
                            socket = Some((wanted, s));
                        }
                        Err(e) => {
                            let mut st = shared.lock();
                            st.bound_port = None;
                            st.error = Some(format!("port {wanted}: {e}"));
                            drop(st);
                            thread::sleep(Duration::from_secs(3));
                            continue;
                        }
                    }
                }
                let Some((_, s)) = &socket else { continue };
                let Ok(len) = s.recv(&mut buffer) else { continue };
                let Ok((_, packet)) = rosc::decoder::decode_udp(&buffer[..len]) else { continue };
                if let Some(id) = avatar_change(&packet) {
                    let mut st = shared.lock();
                    if st.avatar_id.as_deref() != Some(&id) {
                        log::info!("avatar changed to {id}");
                        st.avatar_id = Some(id);
                        st.generation += 1;
                    }
                }
            }
        })
        .expect("spawning OSC receive thread");
    state
}

fn avatar_change(packet: &OscPacket) -> Option<String> {
    match packet {
        OscPacket::Message(m) if m.addr == "/avatar/change" => match m.args.first() {
            Some(OscType::String(id)) => Some(id.clone()),
            _ => None,
        },
        OscPacket::Bundle(b) => b.content.iter().find_map(avatar_change),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vrchat_avatar_config() {
        let text = r#"{"id":"avtr_1","name":"Test","parameters":[
            {"name":"FT/v2/JawOpen","input":{"address":"/avatar/parameters/FT/v2/JawOpen","type":"Float"},
             "output":{"address":"/avatar/parameters/FT/v2/JawOpen","type":"Float"}},
            {"name":"VelocityX","output":{"address":"/avatar/parameters/VelocityX","type":"Float"}},
            {"name":"FT/v2/EyeLid1","input":{"type":"Bool"}}]}"#;
        let info = parse_config(text).unwrap();
        assert_eq!(info.name.as_deref(), Some("Test"));
        assert_eq!(info.params.len(), 3);
        assert_eq!(info.params[2].kind, ParamType::Bool);
    }
}
