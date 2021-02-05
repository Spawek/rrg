use crate::action::finder::chunks::{chunks, Chunks, ChunksConfig};
use crate::action::finder::request::{
    DownloadActionOptions, HashActionOptions,
};
use crate::fs::Entry;
use log::warn;
use rrg_proto::file_finder_download_action_options::OversizedFilePolicy as DownloadOversizedFilePolicy;
use rrg_proto::file_finder_hash_action_options::OversizedFilePolicy as HashOversizedFilePolicy;
use std::fs::File;
use std::io::{BufReader, Read, Take};

#[derive(Debug)]
pub enum Response {
    /// Download action is not performed and no further action is required.
    Skip(),
    /// File was not downloaded, but hash action must be executed.
    HashRequest(HashActionOptions),
    /// Chunks of data to be downloaded.
    DownloadData(Chunks<BufReader<Take<File>>>),
}

/// Performs `download` action logic and returns file contents to be uploaded
/// or another action to be executed.
pub fn download(entry: &Entry, config: &DownloadActionOptions) -> Response {
    if entry.metadata.len() > config.max_size {
        match config.oversized_file_policy {
            DownloadOversizedFilePolicy::Skip => {
                return Response::Skip();
            }
            DownloadOversizedFilePolicy::DownloadTruncated => (),
            DownloadOversizedFilePolicy::HashTruncated => {
                let hash_config = HashActionOptions {
                    max_size: config.max_size,
                    oversized_file_policy:
                        HashOversizedFilePolicy::HashTruncated,
                };
                return Response::HashRequest(hash_config);
            }
        };
    }

    let file = match File::open(&entry.path) {
        Ok(f) => f.take(config.max_size),
        Err(err) => {
            warn!(
                "failed to open file: {}, error: {}",
                entry.path.display(),
                err
            );
            return Response::Skip();
        }
    };

    let reader = BufReader::new(file);
    Response::DownloadData(chunks(
        reader,
        ChunksConfig {
            bytes_per_chunk: config.chunk_size,
            overlap_bytes: 0,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunked_download() {
        let test_string = "some_data";
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("f");
        std::fs::write(&path, &test_string).unwrap();
        let entry = Entry {
            metadata: path.metadata().unwrap(),
            path,
        };

        let result = download(
            &entry,
            &DownloadActionOptions {
                max_size: 100,
                oversized_file_policy: DownloadOversizedFilePolicy::Skip,
                use_external_stores: false,
                chunk_size: 5,
            },
        );

        let mut chunks = match result {
            Response::DownloadData(chunks) => chunks,
            _ => panic!("Unexpected result type."),
        };

        assert_eq!(
            chunks.next().unwrap().unwrap(),
            "some_".bytes().collect::<Vec<_>>()
        );
        assert_eq!(
            chunks.next().unwrap().unwrap(),
            "data".bytes().collect::<Vec<_>>()
        );
        assert!(chunks.next().is_none());
    }

    #[test]
    fn test_no_empty_chunk_download() {
        let test_string = "some_";
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("f");
        std::fs::write(&path, &test_string).unwrap();
        let entry = Entry {
            metadata: path.metadata().unwrap(),
            path,
        };

        let result = download(
            &entry,
            &DownloadActionOptions {
                max_size: 100,
                oversized_file_policy: DownloadOversizedFilePolicy::Skip,
                use_external_stores: false,
                chunk_size: 5,
            },
        );

        let mut chunks = match result {
            Response::DownloadData(chunks) => chunks,
            _ => panic!("Unexpected result type."),
        };

        assert_eq!(
            chunks.next().unwrap().unwrap(),
            "some_".bytes().collect::<Vec<_>>()
        );
        assert!(chunks.next().is_none());
    }

    #[test]
    fn test_skip_above_max_size() {
        let test_string = "some_1";
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("f");
        std::fs::write(&path, &test_string).unwrap();
        let entry = Entry {
            metadata: path.metadata().unwrap(),
            path,
        };

        let result = download(
            &entry,
            &DownloadActionOptions {
                max_size: 5,
                oversized_file_policy: DownloadOversizedFilePolicy::Skip,
                use_external_stores: false,
                chunk_size: 5,
            },
        );

        assert!(matches!(result, Response::Skip()));
    }

    #[test]
    fn test_hash_above_max_size() {
        let test_string = "some_1";
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("f");
        std::fs::write(&path, &test_string).unwrap();
        let entry = Entry {
            metadata: path.metadata().unwrap(),
            path,
        };

        let result = download(
            &entry,
            &DownloadActionOptions {
                max_size: 5,
                oversized_file_policy:
                    DownloadOversizedFilePolicy::HashTruncated,
                use_external_stores: false,
                chunk_size: 5,
            },
        );

        assert!(matches!(
            result,
            Response::HashRequest(HashActionOptions {
                max_size: 5,
                oversized_file_policy: HashOversizedFilePolicy::HashTruncated,
            })
        ));
    }

    #[test]
    fn test_download_truncated_above_max_size() {
        let test_string = "some_1";
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("f");
        std::fs::write(&path, &test_string).unwrap();
        let entry = Entry {
            metadata: path.metadata().unwrap(),
            path,
        };

        let result = download(
            &entry,
            &DownloadActionOptions {
                max_size: 5,
                oversized_file_policy:
                    DownloadOversizedFilePolicy::DownloadTruncated,
                use_external_stores: false,
                chunk_size: 3,
            },
        );

        let mut chunks = match result {
            Response::DownloadData(chunks) => chunks,
            _ => panic!("Unexpected result type."),
        };

        assert_eq!(
            chunks.next().unwrap().unwrap(),
            "som".bytes().collect::<Vec<_>>()
        );
        assert_eq!(
            chunks.next().unwrap().unwrap(),
            "e_".bytes().collect::<Vec<_>>()
        );
        assert!(chunks.next().is_none());
    }
}
