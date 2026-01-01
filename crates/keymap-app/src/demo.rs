//! Demo mode: a temporary copy of the synthetic example configuration.
//! Nothing outside that temporary directory is ever written.

use std::fs;
use std::path::PathBuf;

const FILES: &[(&str, &str)] = &[
    ("config.kdl", include_str!("../../../examples/configs/basic/config.kdl")),
    ("binds-extra.kdl", include_str!("../../../examples/configs/basic/binds-extra.kdl")),
    ("nested/apps.kdl", include_str!("../../../examples/configs/basic/nested/apps.kdl")),
];

pub struct Demo {
    pub dir: PathBuf,
}

impl Demo {
    pub fn create() -> std::io::Result<Demo> {
        let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!("keymap-demo-{}", std::process::id()));
        for (name, text) in FILES {
            let p = dir.join(name);
            fs::create_dir_all(p.parent().unwrap())?;
            fs::write(p, text)?;
        }
        Ok(Demo { dir })
    }

    pub fn config(&self) -> PathBuf {
        self.dir.join("config.kdl")
    }

    pub fn state_dir(&self) -> PathBuf {
        self.dir.join("state")
    }
}

impl Drop for Demo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
