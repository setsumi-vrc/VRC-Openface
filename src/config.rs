use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::i18n::Language;
use crate::processing::{CalibrationStore, ProcessingSettings};
use crate::theme::{CustomColors, Scheme};
use crate::updater::Channel;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Device {
    #[default]
    Auto,
    Cpu,
    Gpu(u32),
}

pub const IDLE_MINUTES: [u32; 7] = [1, 2, 5, 10, 15, 30, 60];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "FT" switch: send tracking to VRChat. Off sends a neutral face.
    pub ft_output: bool,
    /// Closing the window hides it in the system tray instead of quitting.
    pub close_to_tray: bool,
    pub camera_index: u32,
    pub resolution: (u32, u32),
    pub camera_fps: u32,
    pub mirror: bool,
    pub show_camera: bool,
    pub show_landmarks: bool,
    pub cat_ears: bool,
    pub tongue: bool,
    pub show_unused: bool,
    /// The same switch for the list in the Simple view, which starts with the unused ones hidden.
    pub simple_unused: bool,
    /// The full view with the channel tables; off shows the simple one.
    pub advanced: bool,
    pub camera_blur: bool,
    pub debug: bool,
    pub idle_sleep: bool,
    pub idle_minutes: u32,
    pub friendly_names: bool,
    pub interface_scale: f32,
    pub language: Language,
    pub scheme: Scheme,
    pub custom_colors: CustomColors,
    pub osc_host: String,
    pub send_port: u16,
    pub receive_port: u16,
    /// Send every parameter as `FT/v2/<Name>` floats when the avatar is unknown.
    pub send_without_avatar: bool,
    pub interpolation: bool,
    pub face_device: Device,
    pub tongue_device: Device,
    pub update_channel: Channel,
    pub last_avatar_id: Option<String>,
    pub processing: ProcessingSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ft_output: true,
            close_to_tray: true,
            camera_index: 0,
            resolution: (640, 480),
            camera_fps: 30,
            mirror: false,
            show_camera: false,
            show_landmarks: true,
            cat_ears: false,
            tongue: true,
            show_unused: true,
            simple_unused: false,
            advanced: false,
            camera_blur: true,
            debug: false,
            idle_sleep: true,
            idle_minutes: 5,
            friendly_names: true,
            interface_scale: 1.0,
            language: Language::En,
            scheme: Scheme::Default,
            custom_colors: CustomColors::default(),
            osc_host: "127.0.0.1".into(),
            send_port: 9000,
            receive_port: 9001,
            send_without_avatar: true,
            interpolation: true,
            face_device: Device::Auto,
            tongue_device: Device::Auto,
            update_channel: Channel::Stable,
            last_avatar_id: None,
            processing: ProcessingSettings::default(),
        }
    }
}

/// App data folder; `VRC_OPENFACE_DATA` overrides it (useful for tests).
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("VRC_OPENFACE_DATA") {
        return PathBuf::from(dir);
    }
    ProjectDirs::from("", "", "VRC-Openface")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("vrc-openface-data"))
}

/// Imported models live here and take precedence over the bundled folder.
pub fn user_model_dir() -> PathBuf {
    data_dir().join("models")
}

fn load_json<T: DeserializeOwned + Default>(name: &str) -> T {
    fs::read_to_string(data_dir().join(name)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_json<T: Serialize>(name: &str, value: &T) -> Result<()> {
    let dir = data_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    // Write then rename so a crash never leaves a half-written file.
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))
}

impl Settings {
    pub fn load() -> Self {
        let mut s: Settings = load_json("settings.json");
        if !IDLE_MINUTES.contains(&s.idle_minutes) {
            s.idle_minutes = 5;
        }
        s.interface_scale = s.interface_scale.clamp(0.5, 2.0);
        s
    }

    pub fn save(&self) -> Result<()> {
        save_json("settings.json", self)
    }
}

pub fn load_calibrations() -> CalibrationStore {
    load_json("calibrations.json")
}

pub fn save_calibrations(store: &CalibrationStore) -> Result<()> {
    save_json("calibrations.json", store)
}

pub fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// Deletes the whole app data folder.
pub fn erase_all() -> Result<()> {
    let dir = data_dir();
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
    }
    Ok(())
}
