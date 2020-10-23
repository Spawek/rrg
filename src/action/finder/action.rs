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

use crate::session::{self, Session};
use rrg_proto::{FileFinderResult, Hash};
use log::info;
use rrg_proto::path_spec::PathType;
use rrg_proto::file_finder_args::XDev;
use crate::action::finder::request::Action;
use std::fmt::{Formatter, Display};
use crate::action::finder::expand_groups::expand_groups;
use super::request::*;
use std::fs;
use regex::Regex;
use crate::action::finder::path::{PathComponent, build_task_from_path, Task, build_task};

#[derive(Debug)]
pub struct Response {
}

#[derive(Debug)]
struct UnsupportedRequestError {
    message : String,
}

impl Display for UnsupportedRequestError {
    fn fmt(&self, fmt: &mut Formatter) -> std::fmt::Result {
        write!(fmt, "Unsupported client side file finder request error: {}", self.message)
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

pub fn handle<S: Session>(session: &mut S, req: Request) -> session::Result<()> {
    info!("Received client side file finder request request: {:?}", req);

    if req.path_type != PathType::Os {
        return Err(UnsupportedRequestError::new(
            format!("unsupported PathType: {:?}", req.path_type)));
    }

    if req.conditions.len() > 0 {
        return Err(UnsupportedRequestError::new("conditions parameter is not supported".to_string()));
    }

    if req.process_non_regular_files {
        return Err(UnsupportedRequestError::new("process_non_regular_files parameter is not supported".to_string()));
    }

    if req.follow_links {
        return Err(UnsupportedRequestError::new("follow_links parameter is not supported".to_string()));
    }

    if req.xdev_mode != XDev::Local {
        return Err(UnsupportedRequestError::new(format!("unsupported XDev mode: {:?}", req.xdev_mode)));
    }

    if req.paths.len() == 0 {
        return Err(UnsupportedRequestError::new("at least 1 path must be provided".to_string()));
    }

    if req.action.is_none() {
        return Ok(());
    }

    match req.action.unwrap() {
        Action::Stat(_) => {},
        Action::Hash(_) => { return Err(UnsupportedRequestError::new("Hash action is not supported".to_string())) },
        Action::Download(_) => { return Err(UnsupportedRequestError::new("Download action is not supported".to_string())) },
    }
    // TODO: path must be absolute
    // TODO: by default everything is case insensitive
    // TODO: support unicode and non-unicode characters
    // TODO: it would be nice if 1 dir is not scanned twice in the same search - even if paths are overlapping
    //       caching can help

    let tasks: Vec<Task> = req.paths.into_iter()
        .flat_map(|ref x| expand_groups(x))
        .map(|ref x| build_task_from_path(x))
        .collect();

    println!("tasks: {:#?}", tasks);

    let outputs = execute_tasks(tasks);
    println!("resolved paths to: {:#?}", &outputs);

    session.reply(Response {})?;
    Ok(())
}

fn execute_tasks(paths : Vec<Task>) -> Vec<FsObject> {
    // TODO: remove identical elements
    paths.into_iter().flat_map(resolve_path).collect()
}

#[derive(Debug, PartialEq, Clone)]
enum FsObjectType
{
    Dir,
    File  // for now everything that is not a Dir is a file
}

#[derive(Debug, PartialEq, Clone)]
struct FsObject
{
    object_type: FsObjectType,
    path: String
}

fn get_objects_in_path(path: &str) -> Vec<FsObject>
{
    let path = std::path::Path::new(path);
    if !std::path::Path::is_dir(path) {
        if std::path::Path::is_file(path){
            return vec![
                FsObject{
                    path: path.to_str().unwrap().to_owned() /* UNSAFE CALL HERE! */,
                    object_type: FsObjectType::File
                }];
        }
        else {
            return vec![]
        }
        // TODO: support other types? or just switch to the "fs.rs" filetypes.
    }

    let mut ret = vec![];
    for read in fs::read_dir(path){
        for entry in read {
            let entry = entry.unwrap();  // UNSAFE CALL
            let entry_type = if entry.file_type().unwrap().is_dir(){
                FsObjectType::Dir
            }
            else {
                FsObjectType::File
            };
            ret.push(FsObject{
                path: entry.path().to_str().unwrap().to_owned(),
                object_type: entry_type
            });
        }
    }

    println!("FS scan of: {:#?} results: {:#?}", &path, &ret);

    ret
}

#[derive(Debug)]
struct TaskResults {
    new_tasks: Vec<Task>,
    outputs: Vec<FsObject>, // TODO: poor naming - maybe `outputs`? + make it consistent across the code
}

// TODO: change to take path_prefix and remaining components instead of task_details
fn execute_glob_task(regex : &Regex, task_details: &Task) -> TaskResults{
    let mut new_tasks = vec![];
    let mut outputs = vec![];
    for o in get_objects_in_path(&task_details.path_prefix) {
        let relative_path = std::path::Path::strip_prefix(
            std::path::Path::new(&o.path),
            &task_details.path_prefix).unwrap().to_str().unwrap();

        if regex.is_match(relative_path) {
            if task_details.remaining_components.is_empty() {
                outputs.push(o.clone());
            } else {
                let mut new_task_components = vec![];
                new_task_components.push(
                    PathComponent::Constant(task_details.path_prefix.clone()));
                new_task_components.push(
                    PathComponent::Constant(relative_path.to_owned()));
                for x in task_details.remaining_components.clone() {
                    new_task_components.push(x.clone());
                }
                new_tasks.push(build_task(new_task_components));
            }
        }
    }

    TaskResults{ new_tasks, outputs }
}

// TODO: change to take path_prefix and remaining components instead of task_details
fn execute_recursive_scan_task(max_depth : &i32, task_details: &Task) -> TaskResults{
    let mut new_tasks = vec![];

    let mut current_dir_task_components = vec![];
    current_dir_task_components.push(
        PathComponent::Constant(task_details.path_prefix.clone()));
    current_dir_task_components.extend(task_details.remaining_components.clone());
    new_tasks.push(build_task(current_dir_task_components));
    println!("pushed new task: {:#?}", new_tasks.last());

    for o in get_objects_in_path(&task_details.path_prefix) {
        if o.object_type == FsObjectType::Dir {
            let mut new_task_components = vec![];
            new_task_components.push(PathComponent::Constant(o.path.clone()));
            if max_depth > &1 {
                new_task_components.push(PathComponent::RecursiveScan { max_depth: max_depth - 1 });
            }
            new_task_components.extend(task_details.remaining_components.clone());
            new_tasks.push(build_task(new_task_components));
            println!("pushed new task: {:#?}", new_tasks.last());
        }
    }

    TaskResults{ new_tasks, outputs: vec![] }
}

fn execute_constant_task(path: &str) -> TaskResults {
    let path = std::path::Path::new(path);
    dbg!(&path);
    if !path.exists(){
        TaskResults { new_tasks: vec![], outputs: vec![] }
    }
    else {
        let path = path.canonicalize().unwrap();
        if path.is_dir(){
            TaskResults {new_tasks: vec![], outputs: vec![FsObject{object_type: FsObjectType::Dir, path: path.to_str().unwrap().to_owned()}]}
        }
        else {
            TaskResults {new_tasks: vec![], outputs: vec![FsObject{object_type: FsObjectType::File, path: path.to_str().unwrap().to_owned()}]}
            // TODO: support types other than File and Dir
        }
    }
}

fn execute_task(task: &Task) -> TaskResults {
    match &task.current_component {
        PathComponent::Constant(ref c) => {
            execute_constant_task(c)
        },
        PathComponent::Glob(ref regex) => {
            execute_glob_task(regex, &task)
        },
        PathComponent::RecursiveScan { max_depth } => {
            execute_recursive_scan_task(max_depth, &task)
        }
    }
}

// TODO: rename this foo
// TODO: rename arg
fn resolve_path(path: Task) -> Vec<FsObject> {
    let mut tasks = vec![path];
    let mut outputs = vec![];

    while !tasks.is_empty() {
        let task = tasks.swap_remove(tasks.len() - 1);
        println!("--> Working on task: {:?}", &task);

        let mut task_results = execute_task(&task);
        tasks.append(&mut task_results.new_tasks);
        outputs.append(&mut task_results.outputs);

        println!("--> finished task");
    }

    println!("\n!!!resolved path to: {:#?}\n", &outputs);
    outputs
}


impl super::super::Response for Response {

    const RDF_NAME: Option<&'static str> = Some("FileFinderResult");  // ???

    type Proto = FileFinderResult;

    fn into_proto(self) -> FileFinderResult {
        FileFinderResult{
            hash_entry: Some(Hash{
                sha256: Some(vec!(1,2,3,4,5,6,7,8,9,10)),  // TODO: just a test
                ..Default::default()
            }),
            matches: vec!(),
            stat_entry: None,
            transferred_file: None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::finder::path::build_task_from_path;

    #[test]
    fn test_constant_path_with_file() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::write(tempdir.path().join("abc"), "").unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/abc";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: request, object_type: FsObjectType::File });
    }

    #[test]
    fn test_constant_path_with_empty_dir() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        let request = tempdir.path().to_str().unwrap().to_owned() + "/abc";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: request, object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_constant_path_with_nonempty_dir() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abc/def")).unwrap();
        let request = tempdir.path().to_str().unwrap().to_owned() + "/abc";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: request, object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_constant_path_when_file_doesnt_exist() {
        let tempdir = tempfile::tempdir().unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/abc";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 0);
    }


    #[test]
    fn test_constant_path_containing_parent_directory() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("a")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/c")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/a/b/../c";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("a/c").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_glob_star() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abbc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abbd")).unwrap();
        std::fs::create_dir(tempdir.path().join("xbbc")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/a*c";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("abbc").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_glob_star_doesnt_return_intermediate_directories() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("a")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b/c")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/*/*";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("a/b").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_glob_star_followed_by_constant() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abc/123")).unwrap();
        std::fs::create_dir(tempdir.path().join("abc/123/qwe")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/*/123";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("abc/123").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_glob_selection() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("xbc")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/ab[c]";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("abc").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_glob_reverse_selection() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("abe")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/ab[!de]";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("abc").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_glob_wildcard() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("abe")).unwrap();
        std::fs::create_dir(tempdir.path().join("ac")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/a?c";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("abc").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }


    #[test]
    fn test_glob_recurse_default_max_depth() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("a")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b/c")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b/c/d")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/**/c";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("a/b/c").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_glob_recurse_too_low_max_depth_limit() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("a")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b/c")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b/c/d")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/**1/c";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 0);
    }

    #[test]
    fn test_glob_recurse_max_depth() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("a")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b/c")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b/c/d")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/**2/c";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("a/b/c").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_glob_recurse_and_parent_component_in_path() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("a")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b/c")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b/c/d")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/**/../c/d";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("a/b/c/d").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_directory_name_containing_glob_characters() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("a")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b*[xyz]")).unwrap();
        std::fs::create_dir(tempdir.path().join("a/b*[xyz]/c")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/a/*/c";
        let resolved = resolve_path(build_task_from_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("a/b*[xyz]/c").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

// TODO: alternatives tests  // must be done on request level (testing using resolve_path can't cover it)
// TODO: change Path inner type to std::path::Path
// TODO: test with 2 paths reaching identical element
// TODO: test 2 recursive elements throwing an error

    #[test]
    fn local_files_test() {
        let mut session = session::test::Fake::new();
        let request = Request{
            paths: vec!("/home/spaw*/rrg/**1/*toml".to_owned()),
            path_type: PathType::Os,
            action: Some(Action::Stat(StatActionOptions { resolve_links: false, collect_ext_attrs: false } )),
            conditions: vec![],
            process_non_regular_files: false,
            follow_links: false,
            xdev_mode: XDev::Local
        };

        match handle(&mut session, request) {
            Ok(_) => {},
            Err(err) => {panic!("handle error: {}", err)},
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
// TODO: tests on a real FS

// TODO: GRR bug: /home/spawek/rrg/**/*toml doesn't find /home/spawek/rrg/Cargo.toml
// TODO: GRR bug: /home/spawek/rrg/**0/*toml doesn't find /home/spawek/rrg/Cargo.toml