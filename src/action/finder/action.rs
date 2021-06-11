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

// Another differences from GRR:
// for a query '/home/**/*toml' GRR doesn't find /home/Cargo.toml, RRG does
// for a query '/home/**0/*toml' GRR doesn't find /home/Cargo.toml, RRG does


use super::request::*;
use crate::action::finder::chunks::Chunks;
use crate::action::finder::condition::{check_conditions, find_matches};
use crate::action::finder::download;
use crate::action::finder::download::{
    download, Chunk, ChunkId, DownloadEntry,
};
use crate::action::finder::error::Error;
use crate::action::finder::groups::expand_groups;
use crate::action::finder::hash::hash;
use crate::action::finder::resolve::resolve_path;
use crate::action::finder::request::Action;
use crate::action::stat::{
    stat, Request as StatRequest
};
use crate::fs::Entry;
use crate::session::{self, Session};
use log::warn;
use rrg_proto::file_finder_args::XDev;
use std::fs::File;
use std::io::{BufReader, Take};
use std::path::{Path, PathBuf};
use crate::action::finder::response::Response;
use rrg_proto::BufferReference;

/// Handler for the `file_finder` action.
pub fn handle<S: Session>(
    session: &mut S,
    req: Request,
) -> session::Result<()> {
    if req.process_non_regular_files {
        return Err(Error::UnsupportedParameter(
            "process_non_regular_files".to_string(),
        )
            .into());
    }

    if req.xdev_mode != XDev::Local {
        return Err(Error::UnsupportedXDevMode(req.xdev_mode).into());
    }

    let paths = req
        .path_queries
        .iter()
        .flat_map(|x| expand_groups(x))
        .map(into_absoute_path)
        .collect::<session::Result<Vec<_>>>()?;

    for path in paths {
        let entries = resolve_path(&path, req.follow_links)?;
        for entry in entries {
            if !check_conditions(&req.conditions, &entry) {
                continue;
            }

            let matches = find_matches(&req.contents_match_conditions, &entry);
            if req.contents_match_conditions.len() > 0 && matches.is_empty() {
                continue;
            }

            perform_action(session, &entry, &req, matches)?;
        }
    }

    Ok(())
}

/// Performs requested action (`stat`, `hash` or `download`) on
/// the specified entry.
fn perform_action<S: Session>(
    session: &mut S,
    entry: &Entry,
    req: &Request,
    matches: Vec<BufferReference>,
) -> session::Result<()> {
    // `stat` is returned as a part of every request, it's the only part of
    // the response containing file basic file information.
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
                download::Response::Skip() => (),
                download::Response::HashRequest(config) => {
                    if let Some(hash) = hash(&entry, &config) {
                        session.reply(Response::Hash(hash, stat, matches))?;
                    }
                }
                download::Response::DownloadData(chunks) => {
                    let download_entry = upload_chunks(
                        session,
                        chunks,
                        config.chunk_size,
                        &entry.path,
                    )?;

                    if let Some(download_entry) = download_entry {
                        session.reply(Response::Download(
                            download_entry,
                            stat,
                            matches,
                        ))?;
                    }
                }
            },
        },
        None => {
            session.reply(Response::Stat(stat, matches))?;
        }
    };

    Ok(())
}

fn into_absoute_path(s: String) -> session::Result<PathBuf> {
    let path = PathBuf::from(&s);
    if !path.is_absolute() {
        return Err(Error::NonAbsolutePath(path).into());
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

/// Uploads file chunks to the session's transfer store. Returns `None` if
/// reading file fails.
fn upload_chunks<S: Session>(
    session: &mut S,
    chunks: Chunks<BufReader<Take<File>>>,
    chunk_size: u64,
    path: &Path,
) -> session::Result<Option<DownloadEntry>> {
    let mut offset = 0;
    let mut entry = DownloadEntry {
        chunk_size,
        chunk_ids: vec![],
    };
    for chunk in chunks {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                warn!(
                    "reading file: {}, failed after: {} bytes, error: {}",
                    path.display(),
                    offset,
                    err
                );
                return Ok(None);
            }
        };

        entry.chunk_ids.push(ChunkId::make(&chunk, offset));
        offset = offset + chunk.len() as u64;
        session.send(session::Sink::TRANSFER_STORE, Chunk { data: chunk })?;
    }

    Ok(Some(entry))
}

#[cfg(test)]
mod tests {
    use super::*;

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

// TODO: add more tests for the whole action
// TODO: dev type support
// TODO: 2 recursive elements throwing an error
// TODO: do errors like in timeline?
// TODO: make naming "config/options" consistent
// TODO: mby convert paths to (multiple) tasks directly in the request
