use uuid::Uuid;

use crate::contracts::DownloadTaskMeta;
use crate::error::{AppError, AppResult};
use crate::storage::{atomic_fs, StoragePaths};

pub fn load_meta(paths: &StoragePaths, id: Uuid) -> AppResult<DownloadTaskMeta> {
    let path = paths.meta_path(id);
    if !path.exists() {
        return Err(AppError::NotFound(id.to_string()));
    }
    let raw = std::fs::read(&path)?;
    Ok(serde_json::from_slice(&raw)?)
}

pub fn save_meta(paths: &StoragePaths, meta: &DownloadTaskMeta) -> AppResult<()> {
    let json = serde_json::to_vec_pretty(meta)?;
    atomic_fs::write_atomic(&paths.meta_path(meta.id), &json)
}

pub async fn save_meta_async(paths: &StoragePaths, meta: &DownloadTaskMeta) -> AppResult<()> {
    let path = paths.meta_path(meta.id);
    let json = serde_json::to_vec_pretty(meta)?;
    atomic_fs::write_atomic_async(path, json).await
}

pub fn delete_meta(paths: &StoragePaths, id: Uuid) -> AppResult<()> {
    let path = paths.meta_path(id);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub fn list_active_metas(paths: &StoragePaths) -> AppResult<Vec<DownloadTaskMeta>> {
    let dir = paths.active_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut list = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".meta.json"))
        {
            let raw = std::fs::read(&path)?;
            if let Ok(meta) = serde_json::from_slice::<DownloadTaskMeta>(&raw) {
                list.push(meta);
            }
        }
    }
    list.sort_by_key(|a| std::cmp::Reverse(a.created_at));
    Ok(list)
}
