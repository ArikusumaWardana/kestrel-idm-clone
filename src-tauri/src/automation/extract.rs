use std::fs::File;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{AppError, AppResult};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub async fn extract_archive(
    archive_path: PathBuf,
    delete_original: bool,
    seven_zip_bin: Option<PathBuf>,
) -> AppResult<PathBuf> {
    tokio::task::spawn_blocking(move || -> AppResult<PathBuf> {
        let parent = archive_path
            .parent()
            .ok_or_else(|| AppError::ExtractFailed("Archive has no parent directory".into()))?;
        let stem = archive_path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("extracted");
        let dest_dir = parent.join(stem);
        std::fs::create_dir_all(&dest_dir)?;

        let ext = archive_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();

        match ext.as_str() {
            "zip" => extract_zip(&archive_path, &dest_dir)?,
            "7z" => sevenz_rust::decompress_file(&archive_path, &dest_dir)
                .map_err(|e| AppError::ExtractFailed(e.to_string()))?,
            "rar" => extract_with_7z(&archive_path, &dest_dir, seven_zip_bin)?,
            other => {
                return Err(AppError::ExtractFailed(format!(
                    "Unsupported archive format '.{other}' (supported: .zip, .7z, .rar)"
                )));
            }
        }

        if delete_original {
            std::fs::remove_file(&archive_path)?;
        }

        Ok(dest_dir)
    })
    .await
    .map_err(|e| AppError::Other(format!("Archive extraction panicked: {e}")))?
}

fn extract_zip(archive_path: &Path, dest_dir: &Path) -> AppResult<()> {
    let file = File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::ExtractFailed(format!("Invalid ZIP archive: {e}")))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::ExtractFailed(e.to_string()))?;
        let Some(rel_path) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        let out_path = dest_dir.join(rel_path);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(p) = out_path.parent() {
                std::fs::create_dir_all(p)?;
            }
            let mut outfile = File::create(&out_path)?;
            std::io::copy(&mut entry, &mut outfile)?;
        }
    }
    Ok(())
}

fn extract_with_7z(
    archive_path: &Path,
    dest_dir: &Path,
    seven_zip_bin: Option<PathBuf>,
) -> AppResult<()> {
    let bin = seven_zip_bin.ok_or_else(|| AppError::MissingBinary {
        binary: "7z".to_string(),
        hint: "Place 7z-x86_64-pc-windows-msvc.exe in src-tauri/binaries/ to extract .rar files"
            .to_string(),
    })?;

    let output_arg = format!("-o{}", dest_dir.display());
    let output = Command::new(bin)
        .arg("x")
        .arg("-y")
        .arg(output_arg)
        .arg(archive_path)
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::ExtractFailed(format!(
            "7z exited with {}: {}",
            output.status,
            stderr.trim()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    #[tokio::test]
    async fn extracts_zip_and_optionally_deletes_archive() {
        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("bundle.zip");

        {
            let file = File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file("sub/hello.txt", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"hello archive").unwrap();
            writer.finish().unwrap();
        }

        let extracted_dir = extract_archive(zip_path.clone(), true, None).await.unwrap();
        assert!(!zip_path.exists());
        let content = std::fs::read_to_string(extracted_dir.join("sub").join("hello.txt")).unwrap();
        assert_eq!(content, "hello archive");
    }
}
