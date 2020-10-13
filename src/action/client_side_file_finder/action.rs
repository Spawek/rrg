// Copyright 2020 Google LLC
//
// Use of this source code is governed by an MIT-style license that can be found
// in the LICENSE file or at https://opensource.org/licenses/MIT.

//! TODO: add a comment

use crate::session::{self, Session};
use rrg_proto::{FileFinderResult, Hash};
use log::info;
use rrg_proto::path_spec::PathType;
use rrg_proto::file_finder_args::XDev;
use crate::action::client_side_file_finder::request::Action;
use std::fmt::{Formatter, Display};
use crate::action::client_side_file_finder::expand_groups::expand_groups;
use crate::action::client_side_file_finder::path::{Path, parse_path, PathComponent, fold_constant_components};
use super::request::*;
use std::fs;
use regex::Regex;

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
    // I assume the action is Stat after this point.

    // "/path/*" gives all the files from the dir
    // "/path/C*" gives all the files from the dir staring with letter "C"
    // "/path/*md" gives all the files from the dir ending with "md"
    // "/path" or "/path/" stats the dir
    // "/path/*/*" gets all files from 2 directories below
    // path expansion in code: grr_response_client/client_actions/file_finder.py:44

    // recursive component = "**"
    // DEFAULT_MAX_DEPTH = 3
    // change of MAX_DEPTH is done by adding a parameter **<number> - e.g. "**2"

    // TODO: recursive component must be /**\d*/ exactly or it should throw
    // TODO: max 1 recursive component is allowed

    // ? = any char
    // [a-z] works like regexp
    // /home/spawek/rrg/Cargo.[tl]o[cm][lk] -> *toml + *lock

    // TODO: path must be absolute
    // TODO: parent dir is supported? e.g. //asd/asd/../asd
    // TODO: %%user.home%% is implemented on server-side: https://source.corp.google.com/piper///depot/google3/ops/security/grr/core/grr_response_core/lib/interpolation.py - write a comment about it
    // TODO: by default everything is case insensitive
    // TODO: support polish characters

    // Design:
    // Path = [PathComponent]
    // Change request
    // "/home/spawek/rrg/**7/*{t??l, l??k}"
    // to:
    // [Path]: [
    //      [(Constant: "/home/spawek/"), (RecursiveComponent: depth = 7), (Scan: regex = .*t??l)]
    //      [(Constant: "/home/spawek/"), (RecursiveComponent: depth = 7), (Scan: regex = .*l??k)]
    // ]
    // And then map Path to ([Path], [Entry])  // entry = final file/directory path string
    // until the list of paths is empty
    // e.g. mapping (Constant: "/home/spawek/"), (RecursiveComponent: depth = 7), (Scan: regex = .*t??l)
    // may return (
    //  paths[
    //      [(Constant: "/home/spawek/qwe"), (RecursiveComponent: depth = 6), (Scan: regex = .*t??l)])
    //  ],
    //  entries[
    //      ["/home/spawek/file1.toml", "/home/spawek/file2.toml"]
    //  ]
    // )
    //
    // code design
    // create a trait DirReader doing (string -> [file])
    // then:
    // fn Scan(path: Path, dir_reader: DirReader) -> [String]

    // TODO: it would be nice if 1 dir is not scanned twice in the same search - even if paths are overlapping
    // caching can help

    let paths : Vec<Path> = req.paths.into_iter()
        .flat_map(|ref x| expand_groups(x))
        .map(|ref x| parse_path(x))
        .collect();

    println!("paths: {:?}", paths);

    let resolved_paths = resolve_paths(&paths);
    println!("resolved paths: {:?} to: {:?}", &paths, &resolved_paths);

    // for path in req.paths  // TODO: handle a case when a path is inside another one
    // {
    //     let dir = std::fs::read_dir(path);
    //     if !dir.is_ok() {
    //         continue;
    //     }
    //     dir.unwrap().map(|res| res.map(|e| e))
    // }

    session.reply(Response {})?;
    Ok(())
}

fn resolve_paths(paths : &Vec<Path>) -> Vec<FsObject> {
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

    println!("FS scan of: {:?} results: {:?}", &path, &ret);

    ret
}

/// Task is split to parts to make the execution simpler.
#[derive(Debug)]
struct TaskDetails {
    /// Path prefix in which scope the task must be executed.
    /// Given example task: `/a/b/**4/c/d*` this part would be `/a/b`.
    /// Given example task: `/a/b/c` this part would be empty.
    path_prefix: String,

    /// Current `PathComponent` to be executed.
    /// Given example task: `/a/b/**4/c/d*` this part would be `/**4`.
    /// Given example task: `/a/b/c` this part would be `/a/b/c`.
    current_component : PathComponent,

    /// Remaining path components to be executed in following tasks.
    /// Given example task: `/a/b/**4/c/d*` this part would be `c/d*`.
    /// Given example task: `/a/b/c` this part would be empty.
    remaining_components : Vec<PathComponent>,
}

fn get_task_details(task: &Path) -> TaskDetails {
    let folded_components = fold_constant_components(&task.components);
    println!("folded components: {:?}", folded_components);

    // Scan components until getting non-const component or
    // reaching the end of the path.
    let mut path_prefix = "".to_owned();
    for i in 0..folded_components.len(){
        let component = folded_components.get(i).unwrap();
        match component{
            PathComponent::Constant(c) => {
                path_prefix = c.to_owned();
            },
            v @ PathComponent::Glob(_) => {
                let remaining_components = folded_components[i+1..]
                    .into_iter().map(|x| x.to_owned()).collect();
                return TaskDetails{path_prefix, current_component: v.clone(), remaining_components}
            },
            v @ PathComponent::RecursiveScan {..} => {
                let remaining_components = folded_components[i+1..]
                    .into_iter().map(|x| x.to_owned()).collect();
                return TaskDetails{path_prefix, current_component: v.clone(), remaining_components}

            },
        }
    }

    TaskDetails {
        path_prefix: "".to_owned(),
        current_component: PathComponent::Constant(path_prefix.to_owned()),
        remaining_components: vec![]
    }
}

#[derive(Debug)]
struct TaskResults {
    new_tasks: Vec<Path>,
    outputs: Vec<FsObject>, // TODO: poor naming - maybe `outputs`? + make it consistent across the code
}

fn execute_glob_task(regex : &Regex, task_details: &TaskDetails) -> TaskResults{
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
                new_tasks.push(Path {
                    components: new_task_components
                });
            }
        }
    }

    TaskResults{ new_tasks, outputs }
}

fn execute_recursive_scan_task(max_depth : &i32, task_details: &TaskDetails) -> TaskResults{
    let mut new_tasks = vec![];

    let mut current_dir_task_components = vec![];
    current_dir_task_components.push(
        PathComponent::Constant(task_details.path_prefix.clone()));
    for x in task_details.remaining_components.clone() {
        current_dir_task_components.push(x.clone());
    }
    new_tasks.push(Path { components: current_dir_task_components });
    println!("pushed new task: {:?}", new_tasks.last());

    for o in get_objects_in_path(&task_details.path_prefix) {
        if o.object_type == FsObjectType::Dir {
            let mut new_task_components = vec![];
            new_task_components.push(PathComponent::Constant(o.path.clone()));
            if max_depth > &1 {
                new_task_components.push(PathComponent::RecursiveScan { max_depth: max_depth - 1 });
            }
            for x in task_details.remaining_components.clone() {
                new_task_components.push(x.clone());
            }
            new_tasks.push(Path { components: new_task_components });
            println!("pushed new task: {:?}", new_tasks.last());
        }
    }

    TaskResults{ new_tasks, outputs: vec![] }
}

fn execute_constant_task(path: &str) -> TaskResults {
    let new_tasks = vec![];

    let path = std::path::Path::new(path);
    if !path.exists(){
        TaskResults { new_tasks, outputs: vec![] }
    }
    else {
        if path.is_dir(){
            TaskResults {new_tasks, outputs: vec![FsObject{object_type: FsObjectType::Dir, path: path.to_str().unwrap().to_owned()}]}
        }
        else {
            TaskResults {new_tasks, outputs: vec![FsObject{object_type: FsObjectType::File, path: path.to_str().unwrap().to_owned()}]}
            // TODO: support types other than File and Dir
        }
    }
}

fn execute_task(task: &Path) -> TaskResults {
    let task_details = get_task_details(&task);
    match &task_details.current_component {
        PathComponent::Constant(ref c) => {
            execute_constant_task(c)
        },
        PathComponent::Glob(ref regex) => {
            execute_glob_task(regex, &task_details)
        },
        PathComponent::RecursiveScan { max_depth } => {
            execute_recursive_scan_task(max_depth, &task_details)
        }
    }
}

fn resolve_path(path: &Path) -> Vec<FsObject> {
    let mut tasks = vec![path.clone()];
    let mut outputs = vec![];

    while !tasks.is_empty() {
        let task = tasks.swap_remove(tasks.len() - 1);
        println!("--> Working on task: {:?}", &task);

        let mut task_results = execute_task(&task);
        tasks.append(&mut task_results.new_tasks);
        outputs.append(&mut task_results.outputs);

        println!("--> finished task");
    }

    println!("\n!!!resolved path: {:?} to: {:?}\n", &path, &outputs);
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

    #[test]
    fn test_constant_path_with_file() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::write(tempdir.path().join("abc"), "").unwrap();
        let request = tempdir.path().to_str().unwrap().to_owned() + "/abc";
        let resolved = resolve_path(&parse_path(&request));
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: request, object_type: FsObjectType::File });
    }

    #[test]
    fn test_constant_path_with_empty_dir() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        let request = tempdir.path().to_str().unwrap().to_owned() + "/abc";
        let resolved = resolve_path(&parse_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: request, object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_constant_path_with_nonempty_dir() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abc").join("def")).unwrap();
        let request = tempdir.path().to_str().unwrap().to_owned() + "/abc";
        let resolved = resolve_path(&parse_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: request, object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_constant_path_when_file_doesnt_exist() {
        let tempdir = tempfile::tempdir().unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/abc";
        let resolved = resolve_path(&parse_path(&request));

        assert_eq!(resolved.len(), 0);
    }

    #[test]
    fn test_glob_star() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abbc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abbd")).unwrap();
        std::fs::create_dir(tempdir.path().join("xbbc")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/a*c";
        let resolved = resolve_path(&parse_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("abbc").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_glob_star_followed_by_constant() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abc").join("123")).unwrap();
        std::fs::create_dir(tempdir.path().join("abc").join("123").join("qwe")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/*/123";
        let resolved = resolve_path(&parse_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("abbc").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_glob_selection() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("xbc")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/ab[c]";
        let resolved = resolve_path(&parse_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("abc").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

    #[test]
    fn test_glob_reverse_selection() {
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("abe")).unwrap();

        let request = tempdir.path().to_str().unwrap().to_owned() + "/ab[!ce]";
        let resolved = resolve_path(&parse_path(&request));

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], FsObject{path: tempdir.path().join("abd").to_str().unwrap().to_owned(), object_type: FsObjectType::Dir });
    }

// TODO: other glob features tests

// TODO: recurse tests
// TODO: alternatves tests
// TODO: change Path inner type to std::path::Path

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
