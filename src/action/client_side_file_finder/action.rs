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
use regex::Regex;
use crate::action::client_side_file_finder::glob_to_regex::glob_to_regex;
use crate::action::client_side_file_finder::path::{Path, parse_path, PathComponent};
use super::request::*;
use std::fs;

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


    // TODO: glob to regex: https://source.corp.google.com/piper///depot/google3/ops/security/grr/core/grr_response_core/lib/util/compat/fnmatch.py;l=19;bpv=0;bpt=0;rcl=330034281
    // TODO: support "!" globs

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

    let resolved_paths = resolve_paths(paths);

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

fn resolve_paths(paths : Vec<Path>) -> Vec<FsObject> {
    paths.into_iter().flat_map(resolve_path).collect()
}

fn is_path_constant(path: &Path) -> bool {
    if path.components.len() != 1 {
        return false;
    }

    match path.components.first().unwrap() {
        PathComponent::Constant(_) => true,
        _ => false
    }
}

enum FsObjectType
{
    Dir,
    File  // for now everything that is not a Dir is a file
}

struct FsObject
{
    object_type: FsObjectType,
    path: String
}

fn get_objects_in_path(path: String) -> Vec<FsObject>
{
    let path = std::path::Path::new(&path);
    if !std::path::Path::is_dir(path) {
        return vec![
            FsObject{
                path: path.to_str().unwrap().to_owned() /* UNSAFE CALL HERE! */,
                object_type: FsObjectType::File
            }];
    }

    for entry in fs::read_dir(path){
        // let p = entry.path();
    }

    vec![]
}

fn resolve_path(path: Path) -> Vec<FsObject> {
    let mut tasks = vec![path];
    let results = vec![];

    while !tasks.is_empty() {
        let task = tasks.swap_remove(tasks.len() - 1);

        // Scan components until getting non-const component or
        // reaching the end of the path.
        let mut const_part = PathComponent::Constant("".to_owned());
        let mut non_const_component : Option<PathComponent> = None;
        for component in &task.components {
            match component{
                v @ PathComponent::Constant(_) => {
                    const_part = v.clone()
                },
                v @ PathComponent::Glob(_) => {
                    non_const_component = Some(v.clone());
                    break;
                },
                v@ PathComponent::RecursiveScan { .. } => {
                    non_const_component = Some(v.clone());
                    break;
                },
            }
        };
    }

    results
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
    use crate::session::Error;

    #[test]
    fn test() {
        let mut session = session::test::Fake::new();
        let request = Request{
            paths: vec!("/home/spawek/grr/**/*toml".to_owned()),
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
