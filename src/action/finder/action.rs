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

use super::request::*;
use crate::action::finder::groups::expand_groups;
use crate::action::finder::request::Action;
use crate::action::finder::task::{
    build_task, PathComponent, Task, TaskBuilder,
};
use crate::fs::{list_dir, Entry};
use crate::session::{self, Session};
use log::info;
use log::warn;
use regex::Regex;
use rrg_proto::file_finder_args::XDev;
use rrg_proto::path_spec::PathType;
use rrg_proto::{FileFinderResult, Hash};
use std::fmt::{Display, Formatter};
use std::path::Path;

#[derive(Debug)]
pub struct Response {}

#[derive(Debug)]
struct UnsupportedRequestError {
    message: String,
}

impl Display for UnsupportedRequestError {
    fn fmt(&self, fmt: &mut Formatter) -> std::fmt::Result {
        write!(
            fmt,
            "Unsupported client side file finder request error: {}",
            self.message
        )
    }
}

impl UnsupportedRequestError {
    /// Creates a new error indicating that a regex cannot be parsed.
    pub fn new(message: String) -> session::Error {
        session::Error::Action(Box::new(UnsupportedRequestError { message }))
    }
}

impl std::error::Error for UnsupportedRequestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

pub fn handle<S: Session>(
    session: &mut S,
    req: Request,
) -> session::Result<()> {
    info!(
        "Received client side file finder request request: {:?}",
        req
    );

    if req.path_type != PathType::Os {
        return Err(UnsupportedRequestError::new(format!(
            "unsupported PathType: {:?}",
            req.path_type
        )));
    }

    if req.conditions.len() > 0 {
        return Err(UnsupportedRequestError::new(
            "conditions parameter is not supported".to_string(),
        ));
    }

    if req.process_non_regular_files {
        return Err(UnsupportedRequestError::new(
            "process_non_regular_files parameter is not supported".to_string(),
        ));
    }

    if req.follow_links {
        return Err(UnsupportedRequestError::new(
            "follow_links parameter is not supported".to_string(),
        ));
    }

    if req.xdev_mode != XDev::Local {
        return Err(UnsupportedRequestError::new(format!(
            "unsupported XDev mode: {:?}",
            req.xdev_mode
        )));
    }

    if req.paths.len() == 0 {
        return Err(UnsupportedRequestError::new(
            "at least 1 path must be provided".to_string(),
        ));
    }

    if req.action.is_none() {
        return Ok(());
    }

    match req.action.unwrap() {
        Action::Stat(_) => {}
        Action::Hash(_) => {
            return Err(UnsupportedRequestError::new(
                "Hash action is not supported".to_string(),
            ))
        }
        Action::Download(_) => {
            return Err(UnsupportedRequestError::new(
                "Download action is not supported".to_string(),
            ))
        }
    }
    // TODO: path must be absolute
    // TODO: by default everything is case insensitive
    // TODO: support unicode and non-unicode characters
    // TODO: it would be nice if 1 dir is not scanned twice in the same search - even if paths are overlapping
    //       caching can help

    /////////////////// TODO: change tasks into "paths" here so strings are passed to "resolve" function (and it's the one thats testsd)
    let _outputs: Vec<Entry> = req
        .paths
        .into_iter()
        .flat_map(|ref x| expand_groups(x))
        .flat_map(|ref x| resolve_path(x))
        .collect();

    session.reply(Response {})?;
    Ok(())
}

fn resolve_path(path: &str) -> Vec<Entry> {
    let task = build_task(path);
    execute_task(task)
}

fn list_path(path: &Path) -> Vec<Entry> {
    let metadata = match path.metadata() {
        Ok(v) => v,
        Err(err) =>
            {
                warn!("failed to stat '{}': {}", path.display(), err);
                return vec![];
            }
    };

    // TODO: handle symbolic links etc
    if !metadata.is_dir() {
        return vec![Entry {
            path: path.to_owned(),
            metadata,
        }];
    }

    match list_dir(path) {
        Ok(v) => v.collect(),
        Err(err) => {
            warn!("listing directory '{}' failed :{}", path.display(), err);
            vec![]
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

fn resolve_recursive_scan_task(
    max_depth: &i32,
    path_prefix: &Path,
    remaining_components: &Vec<PathComponent>,
) -> TaskResults {
    let mut new_tasks = vec![];

    let scan_curr_dir = TaskBuilder::new()
        .add_constant(&path_prefix)
        .add_components(remaining_components.clone())
        .build();
    new_tasks.push(scan_curr_dir);

    // TODO: does it work properly when remaining components are empty? It can add current dir second time
    for e in list_path(&path_prefix) {
        if e.metadata.is_dir() {
            let mut subdir_scan = TaskBuilder::new().add_constant(&e.path);
            if max_depth > &1 {
                subdir_scan = subdir_scan.add_recursive_scan(max_depth - 1);
            }
            subdir_scan =
                subdir_scan.add_components(remaining_components.clone());
            new_tasks.push(subdir_scan.build());
        }
    }

    TaskResults {
        new_tasks,
        outputs: vec![],
    }
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

    let canonicalized_path = match path.canonicalize() {
        Ok(v) => v,
        Err(err) => {
            warn!("failed canonicalize '{}': {}", path.display(), err);
            return ret;
        }
    };

    ret.outputs.push(Entry {
        path: canonicalized_path,
        metadata,
    });

    ret
}

fn resolve_task(task: Task) -> TaskResults {
    match &task.current_component {
        PathComponent::Constant(path) => resolve_constant_task(path),
        PathComponent::Glob(regex) => resolve_glob_task(
            regex,
            &task.path_prefix,
            &task.remaining_components,
        ),
        PathComponent::RecursiveScan { max_depth } => {
            resolve_recursive_scan_task(
                max_depth,
                &task.path_prefix,
                &task.remaining_components,
            )
        }
    }
}

fn execute_task(task: Task) -> Vec<Entry> {
    let mut tasks = vec![task];
    let mut outputs = vec![];

    while !tasks.is_empty() {
        let task = tasks.swap_remove(tasks.len() - 1);

        let mut task_results = resolve_task(task);
        tasks.append(&mut task_results.new_tasks);
        outputs.append(&mut task_results.outputs);
    }

    outputs
}

impl super::super::Response for Response {
    const RDF_NAME: Option<&'static str> = Some("FileFinderResult");

    type Proto = FileFinderResult;

    fn into_proto(self) -> FileFinderResult {
        FileFinderResult {
            hash_entry: Some(Hash {
                sha256: Some(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]), // TODO: just a test
                ..Default::default()
            }),
            matches: vec![],
            stat_entry: None,
            transferred_file: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_path_with_file() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::write(tempdir.path().join("a"), "").unwrap();

        let request = tempdir.path().join("a");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, request);
        assert!(resolved[0].metadata.is_file());
    }

    #[test]
    fn test_constant_path_with_empty_dir() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("a")).unwrap();

        let request = tempdir.path();
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, request.to_path_buf());
        assert!(resolved[0].metadata.is_dir());
    }

    #[test]
    fn test_constant_path_with_nonempty_dir() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("a");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, request);
    }

    #[test]
    fn test_constant_path_when_file_doesnt_exist() {
        let tempdir = tempfile::tempdir().unwrap();

        let request = tempdir.path().join("abc");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 0);
    }

    #[test]
    fn test_constant_path_containing_parent_directory() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("a")).unwrap();
        std::fs::create_dir(tempdir.path().join("a").join("b")).unwrap();
        std::fs::create_dir(tempdir.path().join("a").join("c")).unwrap();

        let request = tempdir.path().join("a").join("b").join("..").join("c");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("a").join("c"));
    }

    #[test]
    fn test_glob_star() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abbc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abbd")).unwrap();
        std::fs::create_dir(tempdir.path().join("xbbc")).unwrap();

        let request = tempdir.path().join("a*c");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abbc"));
    }

    #[test]
    fn test_glob_star_doesnt_return_intermediate_directories() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b").join("c");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("*").join("*");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("a").join("b"));
    }

    #[test]
    fn test_glob_star_followed_by_constant() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("abc").join("123").join("qwe");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("*").join("123");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abc").join("123"));
    }

    #[test]
    fn test_glob_selection() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("xbc")).unwrap();

        let request = tempdir.path().join("ab[c]");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abc"));
    }

    #[test]
    fn test_glob_reverse_selection() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("abe")).unwrap();

        let request = tempdir.path().join("ab[!de]");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abc"));
    }

    #[test]
    fn test_glob_wildcard() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("abe")).unwrap();
        std::fs::create_dir(tempdir.path().join("ac")).unwrap();

        let request = tempdir.path().join("a?c");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abc"));
    }

    #[test]
    fn test_glob_recurse_default_max_depth() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b").join("c").join("d");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("**").join("c");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].path,
            tempdir.path().join("a").join("b").join("c")
        );
    }

    #[test]
    fn test_glob_recurse_too_low_max_depth_limit() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b").join("c");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("**1").join("c");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 0);
    }

    #[test]
    fn test_glob_recurse_max_depth() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b").join("c").join("d");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("**2").join("c");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].path,
            tempdir.path().join("a").join("b").join("c")
        );
    }

    #[test]
    fn test_glob_recurse_and_parent_component_in_path() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b").join("c").join("d");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("**").join("..").join("c").join("d");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].path,
            tempdir.path().join("a").join("b").join("c").join("d")
        );
    }

    #[test]
    fn test_directory_name_containing_glob_characters() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b*[xyz]").join("c");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("a").join("*").join("c");
        let resolved = resolve_path(request.to_str().unwrap());

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].path,
            tempdir.path().join("a").join("b*[xyz]").join("c")
        );
    }

    // TODO: alternatives tests  // must be done on request level (testing using resolve_path can't cover it)
    // TODO: change Path inner type to std::path::Path
    // TODO: test with 2 paths reaching identical element
    // TODO: test 2 recursive elements throwing an error

    #[test]
    fn local_files_test() {
        let mut session = session::test::Fake::new();
        let request = Request {
            paths: vec!["/home/spaw*/rrg/**1/*toml".to_owned()],
            path_type: PathType::Os,
            action: Some(Action::Stat(StatActionOptions {
                resolve_links: false,
                collect_ext_attrs: false,
            })),
            conditions: vec![],
            process_non_regular_files: false,
            follow_links: false,
            xdev_mode: XDev::Local,
        };

        match handle(&mut session, request) {
            Ok(_) => {}
            Err(err) => panic!("handle error: {}", err),
        }

        // let args : FileFinderArgs = FileFinderArgs{
        //     action: Some(rrg_proto::FileFinderAction{
        //         // action_type: Some(2),
        //         action_type: None,
        //         ..Default::default()}
        //     ),
        //         ..Default::default()};
        //
        // let req = Request2::from_proto(args);
        // println!("req: {:?}", req);
        //
        // assert_eq!(session.reply_count(), 1);
    }
}

// TODO: create a dir for this action and do request/response in separate files from the main logic

// TODO: GRR bug: /home/spawek/rrg/**/*toml doesn't find /home/spawek/rrg/Cargo.toml
// TODO: GRR bug: /home/spawek/rrg/**0/*toml doesn't find /home/spawek/rrg/Cargo.toml
