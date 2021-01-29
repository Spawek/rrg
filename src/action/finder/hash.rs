use crate::action::finder::request::HashActionOptions;
use crate::fs::Entry;
use crypto::digest::Digest;
use log::warn;
use rrg_macro::ack;
use rrg_proto::file_finder_hash_action_options::OversizedFilePolicy;
use rrg_proto::Hash as HashEntry;
use std::cmp::min;
use std::fs::File;
use std::io::Read;

/// Hashes data writen to it using SHA-1, SHA-256 and MD5 algorithms.
struct Hasher {
    sha1: crypto::sha1::Sha1,
    sha256: crypto::sha2::Sha256,
    md5: crypto::md5::Md5,
    num_bytes: usize,
}

impl Hasher {
    pub fn new() -> Hasher {
        Hasher {
            sha1: crypto::sha1::Sha1::new(),
            sha256: crypto::sha2::Sha256::new(),
            md5: crypto::md5::Md5::new(),
            num_bytes: 0,
        }
    }
}

impl std::io::Write for Hasher {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sha1.input(buf);
        self.sha256.input(buf);
        self.md5.input(buf);
        self.num_bytes = self.num_bytes + buf.len();

        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn hash(entry: &Entry, config: &HashActionOptions) -> Option<HashEntry> {
    match config.oversized_file_policy {
        OversizedFilePolicy::Skip => {
            if entry.metadata.len() > config.max_size {
                return None;
            }
        }
        OversizedFilePolicy::HashTruncated => {}
    };

    let file = ack! {
    File::open(&entry.path), error: "failed to open file: {}, error: {}",
        entry.path.display(),
        err}?;
    let mut file = file.take(config.max_size);

    let mut hasher = Hasher::new();
    match std::io::copy(&mut file, &mut hasher) {
        Ok(read_bytes) => {
            let expected_bytes = min(entry.metadata.len(), config.max_size);
            if read_bytes != expected_bytes {
                warn!(
                    "failed to read all data from: {}, {} bytes were read, but {} were expected",
                    entry.path.display(),
                    &read_bytes,
                    expected_bytes
                );
                return None;
            }
        }
        Err(err) => {
            warn!(
                "failed to copy data from: {}. Error: {}",
                entry.path.display(),
                &err
            );
        }
    }

    Some(HashEntry {
        sha1: Some(hasher.sha1.result_str().as_bytes().to_vec()),
        sha256: Some(hasher.sha256.result_str().as_bytes().to_vec()),
        md5: Some(hasher.md5.result_str().as_bytes().to_vec()),
        pecoff_sha1: None,
        pecoff_md5: None,
        pecoff_sha256: None,
        signed_data: vec![],
        num_bytes: Some(hasher.num_bytes as u64),
        source_offset: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rrg_proto::file_finder_hash_action_options::OversizedFilePolicy;

    #[test]
    fn test_hash_values() {
        let test_string = "some_test_data";
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("f");
        std::fs::write(&path, &test_string).unwrap();
        let entry = Entry {
            metadata: path.metadata().unwrap(),
            path: path,
        };

        let result = hash(
            &entry,
            &HashActionOptions {
                max_size: 14,
                oversized_file_policy: OversizedFilePolicy::Skip,
            },
        )
        .unwrap();

        assert_eq!(
            result.sha1.unwrap(),
            "a62a6d5991238ae72d81fe6e4769b3043d9fe670"
                .as_bytes()
                .to_vec()
        );
        assert_eq!(
            result.sha256.unwrap(),
            "d76d85adca8afad205edebc11f9b5086bca75acb512a748bc79660e1346af546"
                .as_bytes()
                .to_vec()
        );
        assert_eq!(
            result.md5.unwrap(),
            "e091b6f1a233049d22d2807fa8086f3f".as_bytes().to_vec()
        );
        assert_eq!(result.num_bytes.unwrap(), 14);
    }

    #[test]
    fn test_trim_file_over_max_size() {
        let test_string = "some_test_data";
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("f");
        std::fs::write(&path, &test_string).unwrap();
        let entry = Entry {
            metadata: path.metadata().unwrap(),
            path,
        };

        let result = hash(
            &entry,
            &HashActionOptions {
                max_size: 10,
                oversized_file_policy: OversizedFilePolicy::HashTruncated,
            },
        )
        .unwrap();

        assert_eq!(result.num_bytes.unwrap(), 10);
    }

    #[test]
    fn test_skip_file_over_max_size() {
        let test_string = "some_test_data";
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("f");
        std::fs::write(&path, &test_string).unwrap();
        let entry = Entry {
            metadata: path.metadata().unwrap(),
            path,
        };

        assert!(hash(
            &entry,
            &HashActionOptions {
                max_size: 10,
                oversized_file_policy: OversizedFilePolicy::Skip
            },
        )
        .is_none());
    }
}
