use rand::RngCore;
use std::path::PathBuf;

use crate::contracts::{AppSettings, RoutingRule};
use crate::error::AppResult;
use crate::storage::{atomic_fs, StoragePaths};

pub fn generate_auth_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn default_downloads_dir() -> String {
    std::env::var_os("USERPROFILE")
        .map(|p| PathBuf::from(p).join("Downloads"))
        .unwrap_or_else(|| PathBuf::from("Downloads"))
        .to_string_lossy()
        .to_string()
}

pub fn default_settings() -> AppSettings {
    AppSettings {
        default_download_dir: default_downloads_dir(),
        default_chunks: 8,
        max_concurrent_downloads: 3,
        global_speed_limit_bps: None,
        bridge_port: 17890,
        local_auth_token: generate_auth_token(),
        autostart_enabled: false,
        auto_throttle_enabled: true,
        auto_throttle_percent: 20,
        auto_throttle_processes: vec![
            "zoom.exe".to_string(),
            "teams.exe".to_string(),
            "discord.exe".to_string(),
            "valorant.exe".to_string(),
            "cs2.exe".to_string(),
        ],
        auto_throttle_on_fullscreen: true,
        routing_rules: default_routing_rules(),
    }
}

fn default_routing_rules() -> Vec<RoutingRule> {
    let base = PathBuf::from(default_downloads_dir());
    vec![
        RoutingRule {
            name: "Archives".to_string(),
            domain_pattern: None,
            filename_regex: None,
            extensions: vec!["zip", "7z", "rar", "tar", "gz", "xz"]
                .into_iter()
                .map(String::from)
                .collect(),
            target_dir: base.join("Archives").to_string_lossy().to_string(),
        },
        RoutingRule {
            name: "Video".to_string(),
            domain_pattern: None,
            filename_regex: None,
            extensions: vec!["mp4", "mkv", "webm", "mov", "avi"]
                .into_iter()
                .map(String::from)
                .collect(),
            target_dir: base.join("Video").to_string_lossy().to_string(),
        },
        RoutingRule {
            name: "Documents".to_string(),
            domain_pattern: None,
            filename_regex: None,
            extensions: vec!["pdf", "docx", "xlsx", "pptx", "txt", "csv"]
                .into_iter()
                .map(String::from)
                .collect(),
            target_dir: base.join("Documents").to_string_lossy().to_string(),
        },
    ]
}

pub fn load_or_init_settings(paths: &StoragePaths) -> AppResult<AppSettings> {
    let path = paths.settings_path();
    if path.exists() {
        let raw = std::fs::read_to_string(&path)?;
        let mut settings: AppSettings = toml::from_str(&raw)?;
        if settings.local_auth_token.trim().is_empty() {
            settings.local_auth_token = generate_auth_token();
            save_settings(paths, &settings)?;
        }
        Ok(settings)
    } else {
        let settings = default_settings();
        save_settings(paths, &settings)?;
        Ok(settings)
    }
}

pub fn save_settings(paths: &StoragePaths, settings: &AppSettings) -> AppResult<()> {
    let toml_str = toml::to_string_pretty(settings)?;
    atomic_fs::write_atomic(&paths.settings_path(), toml_str.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn first_run_generates_token_and_persists_toml() {
        let dir = tempdir().unwrap();
        let paths = StoragePaths::from_base(dir.path().to_path_buf()).unwrap();
        let settings = load_or_init_settings(&paths).unwrap();
        assert_eq!(settings.local_auth_token.len(), 64);
        assert!(paths.settings_path().exists());

        let reloaded = load_or_init_settings(&paths).unwrap();
        assert_eq!(reloaded.local_auth_token, settings.local_auth_token);
    }
}
