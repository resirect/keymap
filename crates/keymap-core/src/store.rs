//! Keymap's own local files: appearance settings, display labels and unbound
//! custom-action templates. Binding truth always lives in the Niri config.

use crate::model::Payload;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback))
}

pub fn config_dir() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config").join("keymap")
}

pub fn state_dir() -> PathBuf {
    xdg_dir("XDG_STATE_HOME", ".local/state").join("keymap")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormFactor {
    #[serde(rename = "60")]
    Sixty,
    #[serde(rename = "75")]
    SeventyFive,
    #[serde(rename = "100")]
    Full,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub form_factor: FormFactor,
    /// Keyboard width as a fraction of the display width.
    pub scale: f64,
    pub blur: bool,
    /// Backdrop dimming, 0.0–0.8.
    pub dim: f64,
    pub assigned_only: bool,
    pub reduced_motion: bool,
    /// The opening ripple and other motion; off means keys simply appear.
    pub animations: bool,
    /// The quiet opening sound.
    pub sounds: bool,
    /// Sound volume, 0.0–1.0.
    pub volume: f64,
    /// Explicit Niri config path; `None` means discover it.
    pub config_path: Option<PathBuf>,
    /// Let `show`/`toggle` open Keymap while a fullscreen window is focused.
    pub open_over_fullscreen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            form_factor: FormFactor::Full,
            scale: 0.55,
            blur: true,
            dim: 0.25,
            assigned_only: false,
            reduced_motion: false,
            animations: true,
            sounds: true,
            volume: 0.5,
            config_path: None,
            open_over_fullscreen: false,
        }
    }
}

impl Settings {
    pub fn clamp(mut self) -> Self {
        self.scale = self.scale.clamp(0.3, 0.95);
        self.dim = self.dim.clamp(0.0, 0.8);
        self.volume = self.volume.clamp(0.0, 1.0);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomAction {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
    pub payload: Payload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub custom_actions: Vec<CustomAction>,
    /// Display labels keyed by canonical payload.
    pub labels: HashMap<String, Label>,
}

impl Store {
    pub fn label_for(&self, canonical: &str, custom_id: Option<&str>) -> Option<(String, Option<String>)> {
        if let Some(id) = custom_id
            && let Some(c) = self.custom_actions.iter().find(|c| c.id == id)
        {
            return Some((c.name.clone(), c.category.clone()));
        }
        self.labels.get(canonical).map(|l| (l.name.clone(), l.category.clone()))
    }

    pub fn new_id(&self) -> String {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let mut id = format!("a{:x}", t.as_millis());
        while self.custom_actions.iter().any(|c| c.id == id) {
            id.push('x');
        }
        id
    }
}

/// Read JSON, falling back to defaults when missing. A corrupt file is moved
/// aside rather than silently overwritten.
pub fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    match fs::read(path) {
        Ok(b) => match serde_json::from_slice(&b) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("keymap: {} is not valid ({e}); using defaults", path.display());
                let _ = fs::rename(path, path.with_extension("json.corrupt"));
                T::default()
            }
        },
        Err(_) => T::default(),
    }
}

/// Atomically write a private file (0600) inside a private directory (0700).
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no parent directory"))?;
    fs::create_dir_all(dir)?;
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    let tmp = dir.join(format!(".{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("file")));
    {
        let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let s = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    write_private(path, &s)
}

pub fn load_settings() -> Settings {
    read_json::<Settings>(&config_dir().join("settings.json")).clamp()
}

pub fn save_settings(s: &Settings) -> std::io::Result<()> {
    write_json(&config_dir().join("settings.json"), s)
}

pub fn load_store() -> Store {
    read_json(&config_dir().join("actions.json"))
}

pub fn save_store(s: &Store) -> std::io::Result<()> {
    write_json(&config_dir().join("actions.json"), s)
}
