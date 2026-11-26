use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

pub fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Other(format!("Path has no parent: {}", path.display())))?;
    std::fs::create_dir_all(parent)?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("state");
    let tmp_path = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));

    let write_res = (|| -> AppResult<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    })();

    if write_res.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }

    write_res
}

pub async fn write_atomic_async(path: PathBuf, bytes: Vec<u8>) -> AppResult<()> {
    tokio::task::spawn_blocking(move || write_atomic(&path, &bytes))
        .await
        .map_err(|e| AppError::Other(format!("Atomic write task panicked: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_atomic_creates_and_overwrites_without_leftover_tmp() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("nested").join("settings.toml");

        write_atomic(&target, b"first = true\n").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"first = true\n");

        write_atomic(&target, b"second = true\n").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"second = true\n");

        let entries: Vec<_> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries, vec!["settings.toml"]);
    }
}
