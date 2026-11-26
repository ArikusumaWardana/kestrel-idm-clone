use std::path::PathBuf;

use crate::contracts::SidecarStatus;

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

fn manifest_binaries_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries")
}

pub fn resolve_sidecar(name: &str) -> Option<PathBuf> {
    let target_triple_name = format!("{name}-x86_64-pc-windows-msvc.exe");
    let plain_name = format!("{name}.exe");

    if let Some(dir) = exe_dir() {
        let p1 = dir.join(&target_triple_name);
        if p1.is_file() {
            return Some(p1);
        }
        let p2 = dir.join(&plain_name);
        if p2.is_file() {
            return Some(p2);
        }
        let p3 = dir.join("binaries").join(&target_triple_name);
        if p3.is_file() {
            return Some(p3);
        }
    }

    let dev_bin = manifest_binaries_dir().join(&target_triple_name);
    if dev_bin.is_file() {
        return Some(dev_bin);
    }
    let dev_plain = manifest_binaries_dir().join(&plain_name);
    if dev_plain.is_file() {
        return Some(dev_plain);
    }

    if let Some(path_var) = std::env::var_os("PATH") {
        for folder in std::env::split_paths(&path_var) {
            let candidate = folder.join(&plain_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

pub fn detect_sidecars() -> SidecarStatus {
    SidecarStatus {
        ytdlp: resolve_sidecar("yt-dlp").is_some(),
        ffmpeg: resolve_sidecar("ffmpeg").is_some(),
        seven_zip: resolve_sidecar("7z").is_some(),
    }
}
