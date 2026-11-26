pub mod atomic_fs;
pub mod history;
pub mod meta;
pub mod settings;

use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct StoragePaths {
    pub base_dir: PathBuf,
}

impl StoragePaths {
    pub fn resolve() -> AppResult<Self> {
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                if exe_dir.join("portable.flag").exists() {
                    return Self::from_base(exe_dir.join("data"));
                }
            }
        }

        let appdata = std::env::var_os("APPDATA").ok_or_else(|| {
            AppError::Other("APPDATA environment variable is not set".to_string())
        })?;
        Self::from_base(PathBuf::from(appdata).join("idm-clone"))
    }

    pub fn from_base(base_dir: PathBuf) -> AppResult<Self> {
        std::fs::create_dir_all(&base_dir)?;
        std::fs::create_dir_all(base_dir.join("active"))?;
        Ok(Self { base_dir })
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    pub fn settings_path(&self) -> PathBuf {
        self.base_dir.join("settings.toml")
    }

    pub fn history_path(&self) -> PathBuf {
        self.base_dir.join("history.json")
    }

    pub fn active_dir(&self) -> PathBuf {
        self.base_dir.join("active")
    }

    pub fn meta_path(&self, id: Uuid) -> PathBuf {
        self.active_dir().join(format!("{id}.meta.json"))
    }
}
