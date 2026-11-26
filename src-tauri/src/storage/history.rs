use uuid::Uuid;

use crate::contracts::HistoryEntry;
use crate::error::AppResult;
use crate::storage::{atomic_fs, StoragePaths};

pub fn load_history(paths: &StoragePaths) -> AppResult<Vec<HistoryEntry>> {
    let path = paths.history_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read(&path)?;
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_slice(&raw)?)
}

pub fn save_history(paths: &StoragePaths, entries: &[HistoryEntry]) -> AppResult<()> {
    let json = serde_json::to_vec_pretty(entries)?;
    atomic_fs::write_atomic(&paths.history_path(), &json)
}

pub fn upsert_history_entry(paths: &StoragePaths, entry: HistoryEntry) -> AppResult<Vec<HistoryEntry>> {
    let mut entries = load_history(paths)?;
    if let Some(existing) = entries.iter_mut().find(|e| e.id == entry.id) {
        *existing = entry;
    } else {
        entries.insert(0, entry);
    }
    save_history(paths, &entries)?;
    Ok(entries)
}

pub fn remove_history_entry(paths: &StoragePaths, id: Uuid) -> AppResult<Vec<HistoryEntry>> {
    let mut entries = load_history(paths)?;
    entries.retain(|e| e.id != id);
    save_history(paths, &entries)?;
    Ok(entries)
}
