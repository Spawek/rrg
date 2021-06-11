use crate::action::finder::hash::FileHash;
use crate::action::finder::download::DownloadEntry;
use crate::action::stat::{
    Response as StatEntry,
};
use rrg_proto::{BufferReference, FileFinderResult};

/// `Hash` and `Download` actions return also StatEntry.
#[derive(Debug)]
pub enum Response {
    Stat(StatEntry, Vec<BufferReference>),
    Hash(FileHash, StatEntry, Vec<BufferReference>),
    Download(DownloadEntry, StatEntry, Vec<BufferReference>),
}

impl super::super::Response for Response {
    const RDF_NAME: Option<&'static str> = Some("FileFinderResult");

    type Proto = FileFinderResult;

    fn into_proto(self) -> FileFinderResult {
        match self {
            Response::Stat(stat, matches) => FileFinderResult {
                matches,
                stat_entry: Some(stat.into_proto()),
                hash_entry: None,
                transferred_file: None,
            },
            Response::Hash(hash, stat, matches) => FileFinderResult {
                matches,
                stat_entry: Some(stat.into_proto()),
                hash_entry: Some(hash.into()),
                transferred_file: None,
            },
            Response::Download(download, stat, matches) => FileFinderResult {
                matches,
                stat_entry: Some(stat.into_proto()),
                hash_entry: None,
                transferred_file: Some(download.into()),
            },
        }
    }
}
