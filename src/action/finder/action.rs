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
    build_task, build_task_from_components, PathComponent, Task, TaskBuilder,
};
use crate::fs::{list_dir, Entry};
use crate::session::{self, Session};
use log::info;
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
    let outputs: Vec<Entry> = req
        .paths
        .into_iter()
        .flat_map(|ref x| expand_groups(x))
        .flat_map(|ref x| resolve_path(x))
        .collect();

    println!("resolved paths to: {:#?}", &outputs);

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
        Err(_) => return vec![], // TODO(spawek): return some kind of error here
    };

    // TODO: handle symbolic links etc
    if !metadata.is_dir() {
        return vec![Entry {
            path: path.to_owned(),
            metadata,
        }];
    }

    // TODO: handle error here
    list_dir(path).unwrap().collect()
}

#[derive(Debug)]
struct TaskResults {
    new_tasks: Vec<Task>,
    outputs: Vec<Entry>,
}

// TODO: change to take path_prefix and remaining components instead of task_details
fn resolve_glob_task(regex: &Regex, task_details: &Task) -> TaskResults {
    let mut new_tasks = vec![];
    let mut outputs = vec![];
    for e in list_path(&task_details.path_prefix) {
        let relative_path = std::path::Path::strip_prefix(
            std::path::Path::new(&e.path),
            &task_details.path_prefix,
        )
        .unwrap();
        let relative_path_str = relative_path.to_str().unwrap(); // TODO: handle an error here

        if regex.is_match(relative_path_str) {
            if task_details.remaining_components.is_empty() {
                outputs.push(e.clone());
            } else {
                let new_task = TaskBuilder::new()
                    .add_constant(&task_details.path_prefix)
                    .add_constant(&relative_path)
                    .add_components(task_details.remaining_components.clone())
                    .build();

                new_tasks.push(new_task);
            }
        }
    }

    TaskResults { new_tasks, outputs }
}

// TODO: change to take path_prefix and remaining components instead of task_details
fn resolve_recursive_scan_task(
    max_depth: &i32,
    task_details: &Task,
) -> TaskResults {
    let mut new_tasks = vec![];

    let mut current_dir_task_components = vec![];
    current_dir_task_components
        .push(PathComponent::Constant(task_details.path_prefix.clone()));
    current_dir_task_components
        .extend(task_details.remaining_components.clone());
    new_tasks.push(build_task_from_components(current_dir_task_components));
    println!("pushed new task: {:#?}", new_tasks.last());

    for o in list_path(&task_details.path_prefix) {
        if o.metadata.is_dir() {
            let mut new_task_components = vec![];
            new_task_components.push(PathComponent::Constant(o.path.clone()));
            if max_depth > &1 {
                new_task_components.push(PathComponent::RecursiveScan {
                    max_depth: max_depth - 1,
                });
            }
            new_task_components
                .extend(task_details.remaining_components.clone());
            new_tasks.push(build_task_from_components(new_task_components));
            println!("pushed new task: {:#?}", new_tasks.last());
        }
    }

    TaskResults {
        new_tasks,
        outputs: vec![],
    }
}

fn resolve_constant_task(path: &Path) -> TaskResults {
    if !path.exists() {
        TaskResults {
            new_tasks: vec![],
            outputs: vec![],
        }
    } else {
        let path = path.canonicalize().unwrap(); // TODO: handle the error
        TaskResults {
            new_tasks: vec![],
            outputs: vec![Entry {
                path: path.to_owned(),
                metadata: path.metadata().unwrap(), // TODO: handle the error
            }],
        }
    }
}

fn resolve_task(task: Task) -> TaskResults {
    match &task.current_component {
        PathComponent::Constant(ref c) => resolve_constant_task(c),
        PathComponent::Glob(ref regex) => resolve_glob_task(regex, &task),
        PathComponent::RecursiveScan { max_depth } => {
            resolve_recursive_scan_task(max_depth, &task)
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

    // TODO: Change FSObject to Łukasz type.

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
