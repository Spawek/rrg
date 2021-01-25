use crate::action::finder::request::{
    DownloadActionOptions, HashActionOptions,
};
use crate::fs::Entry;
use log::warn;
use rrg_proto::file_finder_download_action_options::OversizedFilePolicy as DownloadOversizedFilePolicy;
use rrg_proto::file_finder_hash_action_options::OversizedFilePolicy as HashOversizedFilePolicy;
use std::fs::File;
use std::io::{BufReader, Read, Take};

pub enum Result<R> {
    /// Download action is not performed and no further action is required.
    Skip(),
    /// File was not downloaded, but hash action must be executed.
    HashRequested(HashActionOptions),
    /// Chunks of data to be downloaded.
    DownloadData(Chunks<R>),
}

/// Performs `download` action logic and returns data required for taking
/// next steps.
pub fn download(
    entry: &Entry,
    config: &DownloadActionOptions,
) -> Result<BufReader<Take<File>>> {
    // TODO: how to hide this data type?
    if entry.metadata.len() > config.max_size {
        match config.oversized_file_policy {
            DownloadOversizedFilePolicy::Skip => {
                return Result::Skip();
            }
            DownloadOversizedFilePolicy::DownloadTruncated => {}
            DownloadOversizedFilePolicy::HashTruncated => {
                let hash_config = HashActionOptions {
                    max_size: config.max_size,
                    oversized_file_policy:
                        HashOversizedFilePolicy::HashTruncated,
                };
                return Result::HashRequested(hash_config);
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
            return Result::Skip();
        }
    };
    let reader = BufReader::new(file);
    Result::DownloadData(chunks(reader, config.chunk_size))
}

fn chunks<R: std::io::Read>(reader: R, chunk_size: u64) -> Chunks<R> {
    Chunks {
        bytes: reader.bytes(),
        chunk_size,
    }
}

pub struct Chunks<R> {
    bytes: std::io::Bytes<R>,
    chunk_size: u64,
}

impl<R: std::io::Read> std::iter::Iterator for Chunks<R> {
    type Item = std::io::Result<Vec<u8>>;

    fn next(&mut self) -> Option<std::io::Result<Vec<u8>>> {
        let mut ret = vec![];
        for byte in &mut self.bytes {
            let byte = match byte {
                Ok(byte) => byte,
                Err(err) => return Some(Err(err)),
            };
            ret.push(byte);

            if ret.len() == self.chunk_size as usize {
                return Some(Ok(ret));
            }
        }
        if !ret.is_empty() {
            return Some(Ok(ret));
        }

        return None;
    }
}

// TODO: UTs
