//! Streaming checksums: MD5, SHA-256 and BLAKE3.
//!
//! Files are read once in 1 MiB chunks with a cancellation checkpoint per chunk, so hashing a
//! large file never blocks the UI thread and never defeats Cancel.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use md5::Digest;

use crate::{control::OperationControl, error::OperationError};

const CHUNK_SIZE: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    Md5,
    Sha256,
    Blake3,
}

impl ChecksumAlgorithm {
    /// Stable identifier used by the UI and by persisted settings.
    pub fn id(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha256 => "sha256",
            Self::Blake3 => "blake3",
        }
    }

    pub fn all() -> [Self; 3] {
        [Self::Md5, Self::Sha256, Self::Blake3]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChecksumResult {
    pub path: PathBuf,
    pub algorithm: ChecksumAlgorithm,
    pub hex: String,
    pub bytes: u64,
}

pub fn hash_file(
    path: &Path,
    algorithm: ChecksumAlgorithm,
    control: &OperationControl,
    mut on_bytes: impl FnMut(u64),
) -> Result<ChecksumResult, OperationError> {
    let mut file = fs::File::open(path).map_err(|error| OperationError::from_io(&error))?;
    let mut hasher = Hasher::new(algorithm);
    let mut buffer = vec![0u8; CHUNK_SIZE];
    let mut bytes = 0u64;

    loop {
        control.checkpoint()?;
        let read = file
            .read(&mut buffer)
            .map_err(|error| OperationError::from_io(&error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes += read as u64;
        on_bytes(bytes);
    }

    Ok(ChecksumResult {
        path: path.to_path_buf(),
        algorithm,
        hex: hasher.finish(),
        bytes,
    })
}

enum Hasher {
    Md5(md5::Md5),
    Sha256(sha2::Sha256),
    Blake3(Box<blake3::Hasher>),
}

impl Hasher {
    fn new(algorithm: ChecksumAlgorithm) -> Self {
        match algorithm {
            ChecksumAlgorithm::Md5 => Self::Md5(md5::Md5::new()),
            ChecksumAlgorithm::Sha256 => Self::Sha256(sha2::Sha256::new()),
            ChecksumAlgorithm::Blake3 => Self::Blake3(Box::new(blake3::Hasher::new())),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Self::Md5(hasher) => {
                hasher.update(data);
            }
            Self::Sha256(hasher) => {
                hasher.update(data);
            }
            Self::Blake3(hasher) => {
                hasher.update(data);
            }
        }
    }

    fn finish(self) -> String {
        match self {
            Self::Md5(hasher) => hex(hasher.finalize().as_slice()),
            Self::Sha256(hasher) => hex(hasher.finalize().as_slice()),
            Self::Blake3(hasher) => hasher.finalize().to_hex().to_string(),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(contents: &[u8], algorithm: ChecksumAlgorithm) -> ChecksumResult {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.bin");
        fs::write(&path, contents).unwrap();
        hash_file(&path, algorithm, &OperationControl::new(), |_| {}).unwrap()
    }

    #[test]
    fn md5_matches_the_rfc_1321_vector() {
        let result = hash(b"abc", ChecksumAlgorithm::Md5);
        assert_eq!(result.hex, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(result.bytes, 3);
    }

    #[test]
    fn sha256_matches_the_fips_180_vector() {
        let result = hash(b"abc", ChecksumAlgorithm::Sha256);
        assert_eq!(
            result.hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn empty_input_hashes_are_stable() {
        assert_eq!(
            hash(b"", ChecksumAlgorithm::Md5).hex,
            "d41d8cd98f00b204e9800998ecf8427e"
        );
        assert_eq!(
            hash(b"", ChecksumAlgorithm::Sha256).hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash(b"", ChecksumAlgorithm::Blake3).hex,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn algorithms_have_stable_ids() {
        assert_eq!(ChecksumAlgorithm::Md5.id(), "md5");
        assert_eq!(ChecksumAlgorithm::Sha256.id(), "sha256");
        assert_eq!(ChecksumAlgorithm::Blake3.id(), "blake3");
        assert_eq!(ChecksumAlgorithm::all().len(), 3);
    }

    #[test]
    fn progress_reports_the_final_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.bin");
        let contents = vec![7u8; 2 * CHUNK_SIZE + 17];
        fs::write(&path, &contents).unwrap();

        let mut last = 0;
        hash_file(
            &path,
            ChecksumAlgorithm::Blake3,
            &OperationControl::new(),
            |bytes| {
                last = bytes;
            },
        )
        .unwrap();
        assert_eq!(last, contents.len() as u64);
    }

    #[test]
    fn cancellation_stops_hashing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.bin");
        fs::write(&path, b"abc").unwrap();
        let control = OperationControl::new();
        control.cancel();
        assert_eq!(
            hash_file(&path, ChecksumAlgorithm::Sha256, &control, |_| {}),
            Err(OperationError::Cancelled)
        );
    }

    #[test]
    fn missing_file_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.bin");
        assert_eq!(
            hash_file(
                &path,
                ChecksumAlgorithm::Md5,
                &OperationControl::new(),
                |_| {}
            ),
            Err(OperationError::NotFound)
        );
    }
}
