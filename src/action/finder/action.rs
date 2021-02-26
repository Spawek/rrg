// Copyright 2020 Google LLC
//
// Use of this source code is governed by an MIT-style license that can be found
// in the LICENSE file or at https://opensource.org/licenses/MIT.

//! Defines the handler for `client side file finder` action.
//!
//! The handler keeps a queue of paths to be resolved (a.k.a. `tasks`),
//! initialized by request paths with resolved alternatives.
//! Tasks are resolved by performing filesystem requests and generating
//! outputs or adding new tasks to the queue.

// Symbolic links support.
//
// There are 2 config values describing intended behavior when a symbolic link
// is in path:
//   - `follow_links`: described in the FileFinder proto as:
//     "Should symbolic links be followed in recursive directory listings".
//   - `stat` action config: `resolve_links`: described in Stat action proto as:
//     "If true, the action will yield stat information for link targets,
//      if false, the stat for the link itself will be returned".
//
// However a simple test shows that GRR behavior is different than the one
// intended by `follow_links`. Given a test scenario:
//   -> `/a/file`
//   -> `/b/link_to_a` --> symbolic link to `/a`
// A query: `/b/**/file` (follow_links = false) GRR finds `/b/link_to_a/file`.
// What's interesting - a query: `/b/**` (follow_links = false) GRR
// doesn't find the `/b/link_to_a/file`.
//
// Filesystem traversal in RRG:
//   - Always follow links on constant (e.g. `/b/link_to_a/file`) or
//     glob (e.g. `/b/*/file`) expressions.
//   - When walking thought the filesystem using recursive search
//     (e.g. `/b/**/file`) only follow then symbolic link when
//     `follow_links` is set. The symbolic link itself is also returned if it
//     matches the query - e.g. a query `/b/**` (follow_links = true) returns:
//       -> `/b/link_to_a`
//       -> `/b/link_to_a/file`
//
// RRG behavior on symbolic links when executing actions:
// - `stat` action should follow the link when `resolve_links` is set.
// - `hash` and `download` actions should follow the links.

use super::request::*;
use crate::action::finder::download;
use crate::action::finder::download::download;
use crate::action::finder::groups::expand_groups;
use crate::action::finder::hash::hash;
use crate::action::finder::path::normalize;
use crate::action::finder::request::Action;
use crate::action::finder::task::{
    build_task, PathComponent, Task, TaskBuilder,
};
use crate::action::stat::{
    stat, Request as StatRequest, Response as StatEntry,
};
use crate::fs;
use crate::fs::{list_dir, Entry};
use crate::session::{self, Session, UnsupportedActonParametersError};
use log::{info, warn};
use regex::Regex;
use rrg_proto::file_finder_args::XDev;
use rrg_proto::{FileFinderResult, BufferReference};
use rrg_proto::Hash as HashEntry;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use crate::action::finder::condition::{find_matches, check_conditions};

// TODO: copied from timeline
/// A type representing unique identifier of a given chunk.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ChunkId {
    /// A SHA-256 digest of the referenced chunk data.
    sha256: [u8; 32],
    offset: u64,
    length: u64,
}

impl ChunkId {
    /// Creates a chunk identifier for the given chunk.
    fn make(chunk: &Chunk, offset: u64) -> ChunkId {
        ChunkId {
            sha256: Sha256::digest(&chunk.data).into(),
            length: chunk.data.len() as u64,
            offset,
        }
    }
}

#[derive(Debug)]
struct DownloadEntry {
    chunk_ids: Vec<ChunkId>,
    chunk_size: u64,
}

impl From<DownloadEntry> for rrg_proto::BlobImageDescriptor {
    fn from(e: DownloadEntry) -> rrg_proto::BlobImageDescriptor {
        rrg_proto::BlobImageDescriptor {
            chunks: e
                .chunk_ids
                .into_iter()
                .map(|x| rrg_proto::BlobImageChunkDescriptor {
                    offset: Some(x.offset),
                    length: Some(x.length),
                    digest: Some(x.sha256.to_vec()),
                })
                .collect::<Vec<_>>(),
            chunk_size: Some(e.chunk_size),
        }
    }
}

/// A type representing a particular chunk of the returned timeline.
struct Chunk {
    data: Vec<u8>,
}

impl Chunk {
    /// Constructs a chunk from the given blob of bytes.
    fn from_bytes(data: Vec<u8>) -> Chunk {
        Chunk { data }
    }
}

impl super::super::Response for Chunk {
    const RDF_NAME: Option<&'static str> = Some("DataBlob");

    type Proto = rrg_proto::DataBlob;

    fn into_proto(self) -> rrg_proto::DataBlob {
        self.data.into()
    }
}

#[derive(Debug)]
enum Response {
    Stat(StatEntry, Vec<BufferReference>),
    /// GRR Hash action returns also StatEntry, we keep the same behavior.
    Hash(HashEntry, StatEntry, Vec<BufferReference>),
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
                hash_entry: Some(hash),
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

fn into_absoute_path(s: String) -> session::Result<PathBuf> {
    let path = PathBuf::from(&s);
    if !path.is_absolute() {
        return Err(UnsupportedActonParametersError::new(format!(
            "Non-absolute paths are not supported: {}",
            &s
        ))
        .into());
    }
    Ok(path)
}

fn perform_stat_action(
    e: &Entry,
    config: &StatActionOptions,
) -> session::Result<crate::action::stat::Response> {
    let stat_request = StatRequest {
        path: e.path.to_owned(),
        collect_ext_attrs: config.collect_ext_attrs,
        follow_symlink: config.follow_symlink,
    };

    stat(&stat_request)
}

// TODO: rename?
fn perform_action<S: Session>(
    session: &mut S,
    entry: &Entry,
    req: &Request,
    matches: Vec<BufferReference>,
) -> session::Result<()> {
    // TODO: solve RETURN_OR_WARN pattern somehow
    let stat = match perform_stat_action(entry, &req.stat_options) {
        Ok(v) => v,
        Err(err) => {
            warn!(
                "Stat failed on path: {} error: {}",
                entry.path.display(),
                err
            );
            return Ok(());
        }
    };

    match &req.action {
        Some(action) => match action {
            Action::Hash(config) => {
                if let Some(hash) = hash(&entry, &config) {
                    session.reply(Response::Hash(hash, stat, matches))?;
                }
            }
            Action::Download(config) => match download(&entry, &config) {
                download::Response::Skip() => {}
                download::Response::HashRequest(config) => {
                    if let Some(hash) = hash(&entry, &config) {
                        session.reply(Response::Hash(hash, stat, matches))?;
                    }
                }
                download::Response::DownloadData(chunks) => {
                    let mut offset = 0;
                    let mut response = DownloadEntry {
                        chunk_size: config.chunk_size,
                        chunk_ids: vec![],
                    };
                    for chunk in chunks {
                        let chunk = match chunk {
                            Ok(chunk) => Chunk::from_bytes(chunk),
                            Err(err) => {
                                warn!(
                                    "failed to read file: {}, error: {}",
                                    entry.path.display(),
                                    err
                                );
                                return Ok(());
                            }
                        };

                        response.chunk_ids.push(ChunkId::make(&chunk, offset));
                        offset = offset + chunk.data.len() as u64;
                        session.send(session::Sink::TRANSFER_STORE, chunk)?;
                    }

                    session.reply(Response::Download(response, stat, matches))?;
                }
            },
        },
        None => {
            session.reply(Response::Stat(stat, matches))?;
        }
    };

    Ok(())
}

pub fn handle<S: Session>(
    session: &mut S,
    req: Request,
) -> session::Result<()> {
    // TODO: TMP remove!
    info!("File Finder request: {:#?}", &req);

    if req.process_non_regular_files {
        return Err(UnsupportedActonParametersError::new(
            "process_non_regular_files parameter is not supported".to_string(),
        )
        .into());
    }

    if req.xdev_mode != XDev::Local {
        return Err(UnsupportedActonParametersError::new(format!(
            "unsupported XDev mode: {:?}",
            req.xdev_mode
        ))
        .into());
    }

    let paths = req
        .path_queries
        .iter()
        .flat_map(|x| expand_groups(x))
        .map(into_absoute_path)
        .collect::<session::Result<Vec<_>>>()?;

    let entries = paths.iter().flat_map(|x| resolve_path(x, req.follow_links));

    for entry in entries {
        if !check_conditions(&req.conditions, &entry){
            continue;
        }

        let matches = find_matches(&req.contents_match_conditions, &entry);
        if req.contents_match_conditions.len() > 0 && matches.is_empty() {
            continue;
        }

        perform_action(session, &entry, &req, matches)?;
    }

    Ok(())
}

fn resolve_path(
    path: &Path,
    follow_links: bool,
) -> impl Iterator<Item = Entry> {
    let task = build_task(path);
    ResolvePath {
        outputs: vec![],
        tasks: vec![task],
        follow_links,
    }
}

struct ResolvePath {
    /// Results buffered to be returned.
    outputs: Vec<Entry>,
    /// Remaining tasks to be executed.
    tasks: Vec<Task>,
    /// If true then symbolic links should be followed in recursive scans.
    follow_links: bool,
}

fn normalize_path(e: Entry) -> Entry {
    Entry {
        metadata: e.metadata,
        path: normalize(&e.path),
    }
}

impl std::iter::Iterator for ResolvePath {
    type Item = Entry;

    fn next(&mut self) -> Option<Entry> {
        loop {
            if let Some(v) = self.outputs.pop() {
                return Some(v);
            }

            let task = self.tasks.pop()?;
            let mut task_results = resolve_task(task, self.follow_links);
            self.tasks.append(&mut task_results.new_tasks);
            let outputs = task_results.outputs.into_iter().map(normalize_path);
            self.outputs.extend(outputs);
        }
    }
}

fn resolve_task(task: Task, follow_links: bool) -> TaskResults {
    match &task.current_component {
        PathComponent::Constant(path) => resolve_constant_task(path),
        PathComponent::Glob(regex) => resolve_glob_task(
            regex,
            &task.path_prefix,
            &task.remaining_components,
        ),
        PathComponent::RecursiveScan { max_depth } => {
            resolve_recursive_scan_task(
                *max_depth,
                &task.path_prefix,
                &task.remaining_components,
                follow_links,
            )
        }
    }
}

enum ListPath {
    Next(Option<Entry>),
    ListDir(fs::ListDir),
}

impl std::iter::Iterator for ListPath {
    type Item = Entry;

    fn next(&mut self) -> Option<Entry> {
        match self {
            ListPath::Next(next) => next.take(),
            ListPath::ListDir(iter) => iter.next(),
        }
    }
}

fn list_path(path: &Path) -> impl Iterator<Item = Entry> {
    let metadata = match path.metadata() {
        Ok(v) => v,
        Err(err) => {
            warn!("failed to stat '{}': {}", path.display(), err);
            return ListPath::Next(None);
        }
    };

    if !metadata.is_dir() {
        ListPath::Next(Some(Entry {
            path: path.to_owned(),
            metadata,
        }));
    }

    match list_dir(path) {
        Ok(v) => ListPath::ListDir(v),
        Err(err) => {
            warn!("listing directory '{}' failed :{}", path.display(), err);
            ListPath::Next(None)
        }
    }
}

#[derive(Debug)]
struct TaskResults {
    new_tasks: Vec<Task>,
    outputs: Vec<Entry>,
}

fn last_component_matches(regex: &Regex, path: &Path) -> bool {
    let last_component = match path.components().last() {
        Some(v) => v,
        None => {
            warn!(
                "failed to fetch last component from path: {}",
                path.display()
            );
            return false;
        }
    };

    let last_component = match last_component.as_os_str().to_str() {
        Some(v) => v,
        None => {
            warn!(
                "failed to convert last component of the path to string: {}",
                path.display()
            );
            return false;
        }
    };

    regex.is_match(last_component)
}

fn resolve_glob_task(
    glob: &Regex,
    path_prefix: &Path,
    remaining_components: &Vec<PathComponent>,
) -> TaskResults {
    let mut new_tasks = vec![];
    let mut outputs = vec![];
    for e in list_path(&path_prefix) {
        if last_component_matches(&glob, &e.path) {
            if remaining_components.is_empty() {
                outputs.push(e.clone());
            } else {
                let new_task = TaskBuilder::new()
                    .add_constant(&e.path)
                    .add_components(remaining_components.clone())
                    .build();
                new_tasks.push(new_task);
            }
        }
    }

    TaskResults { new_tasks, outputs }
}

/// Checks if Entry is a directory using the symlink_metadata it contains,
/// and metadata if `follow_links` is set.
fn is_dir(e: &Entry, follow_links: bool) -> bool {
    if e.metadata.is_dir() {
        return true;
    }

    if follow_links {
        match std::fs::metadata(&e.path) {
            Ok(metadata) => {
                return metadata.is_dir();
            }
            Err(err) => {
                warn!("failed to stat '{}': {}", e.path.display(), err);
                return false;
            }
        }
    }

    return false;
}

fn resolve_recursive_scan_task(
    max_depth: i32,
    path_prefix: &Path,
    remaining_components: &Vec<PathComponent>,
    follow_links: bool,
) -> TaskResults {
    let mut new_tasks = vec![];
    let mut outputs = vec![];
    for e in list_path(&path_prefix) {
        if !is_dir(&e, follow_links) {
            if remaining_components.is_empty() {
                outputs.push(e.to_owned());
            }
            continue;
        }

        let subdir_scan = TaskBuilder::new()
            .add_constant(&e.path)
            .add_components(remaining_components.clone())
            .build();
        new_tasks.push(subdir_scan);

        if max_depth > 1 {
            let mut recursive_scan = TaskBuilder::new().add_constant(&e.path);
            recursive_scan = recursive_scan.add_recursive_scan(max_depth - 1);
            recursive_scan =
                recursive_scan.add_components(remaining_components.clone());
            new_tasks.push(recursive_scan.build());
        }
    }

    TaskResults { new_tasks, outputs }
}

fn resolve_constant_task(path: &Path) -> TaskResults {
    let mut ret = TaskResults {
        new_tasks: vec![],
        outputs: vec![],
    };

    if !path.exists() {
        return ret;
    }

    let metadata = match path.metadata() {
        Ok(v) => v,
        Err(err) => {
            warn!("failed to stat '{}': {}", path.display(), err);
            return ret;
        }
    };

    ret.outputs.push(Entry {
        path: path.to_owned(),
        metadata,
    });

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_path_with_file() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::write(tempdir.path().join("a"), "").unwrap();

        let request = tempdir.path().join("a");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, request);
        assert!(resolved[0].metadata.is_file());
    }

    #[test]
    fn test_constant_path_with_empty_dir() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("a")).unwrap();

        let request = tempdir.path();
        let resolved = resolve_path(request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, request.to_path_buf());
        assert!(resolved[0].metadata.is_dir());
    }

    #[test]
    fn test_constant_path_with_nonempty_dir() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("a");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, request);
    }

    #[test]
    fn test_constant_path_when_file_doesnt_exist() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();

        let request = tempdir.path().join("abc");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 0);
    }

    #[test]
    fn test_constant_path_containing_parent_directory() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("a")).unwrap();
        std::fs::create_dir(tempdir.path().join("a").join("b")).unwrap();
        std::fs::create_dir(tempdir.path().join("a").join("c")).unwrap();

        let request = tempdir.path().join("a").join("b").join("..").join("c");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("a").join("c"));
    }

    #[test]
    fn test_glob_star() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abbc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abbd")).unwrap();
        std::fs::create_dir(tempdir.path().join("xbbc")).unwrap();

        let request = tempdir.path().join("a*c");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abbc"));
    }

    #[test]
    fn test_glob_star_doesnt_return_intermediate_directories() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b").join("c");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("*").join("*");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("a").join("b"));
    }

    #[test]
    fn test_glob_star_followed_by_constant() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("abc").join("123").join("qwe");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("*").join("123");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abc").join("123"));
    }

    #[test]
    fn test_glob_selection() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("xbc")).unwrap();

        let request = tempdir.path().join("ab[c]");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abc"));
    }

    #[test]
    fn test_glob_reverse_selection() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("abe")).unwrap();

        let request = tempdir.path().join("ab[!de]");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abc"));
    }

    #[test]
    fn test_glob_wildcard() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("abe")).unwrap();
        std::fs::create_dir(tempdir.path().join("ac")).unwrap();

        let request = tempdir.path().join("a?c");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abc"));
    }

    #[test]
    fn test_glob_recurse_default_max_depth() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b").join("c").join("d");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("**").join("c");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].path,
            tempdir.path().join("a").join("b").join("c")
        );
    }

    #[test]
    fn test_glob_recurse_too_low_max_depth_limit() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b").join("c");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("**1").join("c");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 0);
    }

    #[test]
    fn test_glob_recurse_at_the_end_of_the_path() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let a = tempdir.path().join("a");
        std::fs::create_dir(&a).unwrap();
        let file = a.join("file");
        std::fs::write(&file, "").unwrap();

        let request = tempdir.path().join("**");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 2);
        assert!(resolved.iter().find(|x| x.path == a).is_some());
        assert!(resolved.iter().find(|x| x.path == file).is_some());
    }

    #[test]
    fn test_glob_recurse_max_depth() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b").join("c").join("d");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("**2").join("c");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].path,
            tempdir.path().join("a").join("b").join("c")
        );
    }

    #[test]
    fn test_glob_recurse_and_parent_component_in_path() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("**").join("..").join("b");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("a").join("b"));
    }

    #[test]
    fn test_directory_name_containing_glob_characters() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b*[xyz]").join("c");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("a").join("*").join("c");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].path,
            tempdir.path().join("a").join("b*[xyz]").join("c")
        );
    }

    #[test]
    #[cfg(target_family = "unix")]
    fn test_resolve_link_in_const_path() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();

        let a = tempdir.path().join("a");
        std::fs::create_dir(&a).unwrap();
        std::fs::write(a.join("file"), "").unwrap();

        let symlink = tempdir.path().join("b");
        std::os::unix::fs::symlink(&a, &symlink).unwrap();

        {
            let request = symlink.to_owned();
            let resolved =
                resolve_path(&request, follow_links).collect::<Vec<_>>();
            assert_eq!(resolved.len(), 1);
            assert_eq!(resolved[0].path, symlink);
        }

        {
            let request = symlink.join("file").to_owned();
            let resolved =
                resolve_path(&request, follow_links).collect::<Vec<_>>();
            assert_eq!(resolved.len(), 1);
            assert_eq!(resolved[0].path, symlink.join("file"));
        }
    }

    #[test]
    #[cfg(target_family = "unix")]
    fn test_resolve_link_in_glob() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();

        let a = tempdir.path().join("a");
        std::fs::create_dir(&a).unwrap();
        std::fs::write(a.join("file"), "").unwrap();

        let b = tempdir.path().join("b");
        std::fs::create_dir(&b).unwrap();
        let symlink = b.join("link_to_a");
        std::os::unix::fs::symlink(&a, &symlink).unwrap();

        let request = tempdir.path().join("b").join("*").join("file");

        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, symlink.join("file"));
    }

    #[test]
    #[cfg(target_family = "unix")]
    fn test_resolve_link_in_recursive_search_with_no_follow() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();

        let a = tempdir.path().join("a");
        std::fs::create_dir(&a).unwrap();
        std::fs::write(a.join("file"), "").unwrap();

        let b = tempdir.path().join("b");
        std::fs::create_dir(&b).unwrap();
        let symlink = b.join("link_to_a");
        std::os::unix::fs::symlink(&a, &symlink).unwrap();

        let request = b.join("**");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, symlink);
    }

    #[test]
    #[cfg(target_family = "unix")]
    fn test_resolve_link_in_recursive_search_with_follow() {
        let follow_links = true;
        let tempdir = tempfile::tempdir().unwrap();

        let a = tempdir.path().join("a");
        std::fs::create_dir(&a).unwrap();
        std::fs::write(a.join("file"), "").unwrap();

        let b = tempdir.path().join("b");
        std::fs::create_dir(&b).unwrap();
        let symlink = b.join("link_to_a");
        std::os::unix::fs::symlink(&a, &symlink).unwrap();

        let request = b.join("**");
        let resolved = resolve_path(&request, follow_links).collect::<Vec<_>>();

        assert_eq!(resolved.len(), 2);
        assert!(resolved.iter().find(|x| x.path == symlink).is_some());
        assert!(resolved
            .iter()
            .find(|x| x.path == symlink.join("file"))
            .is_some());
    }

    #[test]
    fn test_alternatives() {
        let tempdir = tempfile::tempdir().unwrap();
        let f1 = tempdir.path().join("f1");
        std::fs::write(&f1, "").unwrap();
        let f2 = tempdir.path().join("f2");
        std::fs::write(&f2, "").unwrap();

        let mut session = session::test::Fake::new();
        let request = Request {
            path_queries: vec![tempdir
                .path()
                .join("{f1,f2}")
                .to_str()
                .unwrap()
                .to_owned()],
            stat_options: StatActionOptions {
                follow_symlink: false,
                collect_ext_attrs: false,
            },
            action: None,
            conditions: vec![],
            contents_match_conditions: vec![],
            process_non_regular_files: false,
            follow_links: false,
            xdev_mode: XDev::Local,
        };

        match handle(&mut session, request) {
            Ok(_) => {}
            Err(err) => panic!("handle error: {}", err),
        }

        let replies = session.replies().collect::<Vec<&Response>>();
        assert!(replies
            .iter()
            .find(|response| {
                if let Response::Stat(entry, _) = response {
                    entry.path == f1
                } else {
                    false
                }
            })
            .is_some());
        assert!(replies
            .iter()
            .find(|response| {
                if let Response::Stat(entry, _) = response {
                    entry.path == f2
                } else {
                    false
                }
            })
            .is_some());
    }
}

// TODO: GRR bug: /home/spawek/rrg/**/*toml doesn't find /home/spawek/rrg/Cargo.toml
// TODO: GRR bug: /home/spawek/rrg/**0/*toml doesn't find /home/spawek/rrg/Cargo.toml

// TODO: conditions - important:
//     fill matches for `contents_regex_match` and `contents_literal_match` conditions

// TODO: dev type support
// TODO: cleanup function order here
// TODO: 2 recursive elements throwing an error
// TODO: do errors like in timeline?
// TODO: make naming "config/options" consistent
// TODO: resolve path to another file
// TODO: response to separate file

// TODO: mby convert paths to (multiple) tasks directly in the request
