// Copyright 2020 Google LLC
//
// Use of this source code is governed by an MIT-style license that can be found
// in the LICENSE file or at https://opensource.org/licenses/MIT.

//! TODO: add a comment

use lazy_static::lazy_static;
use crate::session::{self, Session, RegexParseError};
use rrg_proto::{FileFinderResult, Hash};
use log::info;
use rrg_proto::path_spec::PathType;
use rrg_proto::file_finder_args::XDev;
use crate::action::client_side_file_finder::request::Action;
use std::fmt::{Formatter, Display};
use crate::action::client_side_file_finder::expand_groups::expand_groups;
use regex::Regex;
use regex::internal::Input;
use std::cmp::max;
use crate::action::client_side_file_finder::glob_to_regex::glob_to_regex;

type Request = crate::action::client_side_file_finder::request::Request;

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

// Correct Path can't contain 2 consecutive `Constant` components.
struct Path {
    components : Vec<PathComponent>
}

enum PathComponent {
    Constant(String),  // e.g. `/home/spawek/`
    Glob(Regex),  // converted from glob e.g. `sp*[wek]??`
    RecursiveComponent{max_depth: i32},  // converted from glob recursive component i.e. `**`
}

fn get_recursive_component(s : &str) -> Option<PathComponent>{
    lazy_static!{
        static ref RE : Regex = Regex::new(r"\*\*(?P<max_depth>\d*)").unwrap();
    }

    match RE.captures(s){
        Some(m) => {
            let max_depth = m["max_depth"].parse::<i32>();
            if max_depth.is_err(){
                return None;  // TODO: throw some error
            }
            Some(PathComponent::RecursiveComponent{max_depth: max_depth.unwrap()})
        }
        None => None
    }

    // TODO: throw ValueError("malformed recursive component") when there is something more in the match
}

fn get_scan_component(s : &str) -> Option<PathComponent>{
    lazy_static!{
        static ref RE : Regex = Regex::new(r"\*|\?|\[.+\]").unwrap();
    }

    if !RE.is_match(s){
        return None;
    }

    match glob_to_regex(s){
        Ok(regex) => Some(PathComponent::Glob(regex)),
        Err(_) => None,  // TODO: handle error case somehow
    }
}

fn get_path_component(s : &str) -> PathComponent {
    let recursive_component = get_recursive_component(s);
    if recursive_component.is_some(){
        return recursive_component.unwrap();
    }

    let scan = get_scan_component(s);
    if scan.is_some(){
        return scan.unwrap();
    }

    PathComponent::Constant(s.to_owned())
}

fn is_constant_component(component: &PathComponent) -> bool {
    match component{
        PathComponent::Constant(_) => true,
        _ => false
    }
}

fn get_constant_component_value(constant_component: &PathComponent) -> String {
    match constant_component{
        PathComponent::Constant(s) => s.to_owned(),
        _ => panic!()
    }
}

fn fold_consecutive_constant_components(components: Vec<PathComponent>) -> Vec<PathComponent>{
    let mut ret = vec![];
    for c in components {
        if !ret.is_empty() && is_constant_component(ret.last().unwrap()) && is_constant_component(&c) {
            let prev_last = ret.swap_remove(ret.len() - 1);
            ret.push(PathComponent::Constant(get_constant_component_value(&prev_last) + &get_constant_component_value(&c)));
        }
        else {
            ret.push(c);
        }
    }

    ret
}

fn parse_path(path: &str) -> Path {
    let split : Vec<&str> = path.split("/").collect();  // TODO: support different OS separators
    let components : Vec<_> = split.into_iter()
        .filter(|x| !x.is_empty())
        .map(get_path_component)
        .collect();

    Path{components:fold_consecutive_constant_components(f)}
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
    // use super::*;

    #[test]
    fn test() {
        // let mut session = session::test::Fake::new();
        // let request = Request2{paths: vec!("SOME_PATH".to_owned()), action: Some(Action::Stat{})};
        // assert!(handle(&mut session, request).is_ok());
        //
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

        // assert_eq!(session.reply_count(), 1);
    }
}

// TODO: create a dir for this action and do request/response in separate files from the main logic
// TODO: tests on a real FS
