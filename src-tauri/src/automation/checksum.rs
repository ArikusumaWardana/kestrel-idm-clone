use md5::Md5;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::PathBuf;

use crate::contracts::{ChecksumAlgorithm, ChecksumStatus};
use crate::error::{AppError, AppResult};

pub async fn verify_file_checksum(
    path: PathBuf,
    expected_hex: String,
    algorithm: ChecksumAlgorithm,
) -> AppResult<ChecksumStatus> {
    let expected_norm = expected_hex.trim().to_ascii_lowercase();
    if expected_norm.is_empty() {
        return Ok(ChecksumStatus::None);
    }

    tokio::task::spawn_blocking(move || -> AppResult<ChecksumStatus> {
        let mut file = std::fs::File::open(&path)?;
        let mut buf = [0u8; 64 * 1024];

        let actual = match algorithm {
            ChecksumAlgorithm::Sha256 => {
                let mut hasher = Sha256::new();
                loop {
                    let n = file.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                hex::encode(hasher.finalize())
            }
            ChecksumAlgorithm::Md5 => {
                let mut hasher = Md5::new();
                loop {
                    let n = file.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                hex::encode(hasher.finalize())
            }
        };

        if actual == expected_norm {
            Ok(ChecksumStatus::Valid)
        } else {
            Ok(ChecksumStatus::Mismatch)
        }
    })
    .await
    .map_err(|e| AppError::Other(format!("Checksum verification panicked: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[tokio::test]
    async fn verifies_sha256_and_md5_valid_and_mismatch() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"idm-clone-checksum-test").unwrap();
        let path = tmp.path().to_path_buf();

        let expected_sha = hex::encode(Sha256::digest(b"idm-clone-checksum-test"));
        let res = verify_file_checksum(path.clone(), expected_sha, ChecksumAlgorithm::Sha256)
            .await
            .unwrap();
        assert_eq!(res, ChecksumStatus::Valid);

        let mismatch = verify_file_checksum(
            path.clone(),
            "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            ChecksumAlgorithm::Sha256,
        )
        .await
        .unwrap();
        assert_eq!(mismatch, ChecksumStatus::Mismatch);

        let expected_md5 = hex::encode(Md5::digest(b"idm-clone-checksum-test"));
        let res_md5 = verify_file_checksum(path, expected_md5, ChecksumAlgorithm::Md5)
            .await
            .unwrap();
        assert_eq!(res_md5, ChecksumStatus::Valid);
    }
}
