use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};
use xxhash_rust::xxh64::Xxh64;

use super::VaultError;

const STREAM_CHUNK_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChecksums {
    pub xxhash64: String,
    pub sha256: String,
    pub bytes_hashed: u64,
}

/// Computes `xxhash64` and `sha256` in a single streaming pass over `reader`.
pub fn compute_reader_checksums<R: Read>(mut reader: R) -> Result<FileChecksums, VaultError> {
    let mut xx = Xxh64::new(0);
    let mut sha = Sha256::new();
    let mut buf = vec![0u8; STREAM_CHUNK_BYTES];
    let mut total = 0u64;

    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let slice = &buf[..n];
                xx.update(slice);
                sha.update(slice);
                total = total.saturating_add(n as u64);
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => {
                return Err(VaultError::Io {
                    context: "reading stream for checksum computation".to_string(),
                    source: err,
                });
            }
        }
    }

    Ok(FileChecksums {
        xxhash64: format!("{:016x}", xx.digest()),
        sha256: hex::encode(sha.finalize()),
        bytes_hashed: total,
    })
}

pub fn compute_file_checksums(path: &Path) -> Result<FileChecksums, VaultError> {
    let file = File::open(path).map_err(|err| VaultError::Io {
        context: format!("opening '{}' for checksum computation", path.display()),
        source: err,
    })?;
    compute_reader_checksums(file)
}

pub fn verify_file_checksums(
    path: &Path,
    expected_xxhash64: &str,
    expected_sha256: &str,
    item_id: &str,
) -> Result<FileChecksums, VaultError> {
    let actual = compute_file_checksums(path)?;
    if !actual.xxhash64.eq_ignore_ascii_case(expected_xxhash64) {
        return Err(VaultError::ChecksumMismatch {
            item_id: item_id.to_string(),
            algorithm: "xxhash64",
            expected: expected_xxhash64.to_string(),
            actual: actual.xxhash64,
        });
    }
    if !actual.sha256.eq_ignore_ascii_case(expected_sha256) {
        return Err(VaultError::ChecksumMismatch {
            item_id: item_id.to_string(),
            algorithm: "sha256",
            expected: expected_sha256.to_string(),
            actual: actual.sha256,
        });
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn computes_known_xxhash64_and_sha256_streaming() {
        let mut tmp = NamedTempFile::new().unwrap();
        let payload = b"WinDirStat-LiquidGlass-Vault-Payload-1234567890".repeat(8192);
        tmp.write_all(&payload).unwrap();
        tmp.flush().unwrap();

        let digest = compute_file_checksums(tmp.path()).unwrap();
        assert_eq!(digest.bytes_hashed, payload.len() as u64);
        assert_eq!(digest.xxhash64.len(), 16);
        assert_eq!(digest.sha256.len(), 64);

        let verified =
            verify_file_checksums(tmp.path(), &digest.xxhash64, &digest.sha256, "item-1").unwrap();
        assert_eq!(verified, digest);
    }

    #[test]
    fn known_empty_and_abc_vectors_match_standard_specifications() {
        let empty = compute_reader_checksums(io::empty()).unwrap();
        assert_eq!(empty.xxhash64, "ef46db3751d8e999");
        assert_eq!(
            empty.sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        let abc = compute_reader_checksums(io::Cursor::new(b"abc")).unwrap();
        assert_eq!(
            abc.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn verify_detects_xxhash64_and_sha256_mismatch() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"immutable-bytes").unwrap();
        tmp.flush().unwrap();

        let digest = compute_file_checksums(tmp.path()).unwrap();
        let err_xx =
            verify_file_checksums(tmp.path(), "0000000000000000", &digest.sha256, "item-xx")
                .unwrap_err();
        assert!(matches!(
            err_xx,
            VaultError::ChecksumMismatch {
                algorithm: "xxhash64",
                ..
            }
        ));

        let err_sha = verify_file_checksums(
            tmp.path(),
            &digest.xxhash64,
            &"f".repeat(64),
            "item-sha",
        )
        .unwrap_err();
        assert!(matches!(
            err_sha,
            VaultError::ChecksumMismatch {
                algorithm: "sha256",
                ..
            }
        ));
    }
}
