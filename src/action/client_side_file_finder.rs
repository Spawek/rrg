// Copyright 2020 Google LLC
//
// Use of this source code is governed by an MIT-style license that can be found
// in the LICENSE file or at https://opensource.org/licenses/MIT.

//! TODO: add a comment
//!
//! TODO: how should % be handled? how does it support multiple files?  TODO: find example query with multiple returns
//! TODO: why path_spec::PathType (and other enums) are not resolved in proto, but an int is returned?
//!     handle it somehow

use crate::session::{self, Session};
use rrg_proto::{FileFinderArgs, FileFinderResult, Hash, FileFinderAction, FileFinderStatActionOptions, FileFinderHashActionOptions, FileFinderDownloadActionOptions};
use log::info;

type ActionType = rrg_proto::file_finder_action::Action;
type HashActionOversizedFilePolicy = rrg_proto::file_finder_hash_action_options::OversizedFilePolicy;
type DownloadActionOversizedFilePolicy = rrg_proto::file_finder_download_action_options::OversizedFilePolicy;

#[derive(Debug)]
pub struct Request2 {  // TODO: rename to Request
    paths: Vec<String>,
    action: Option<Action>
}

#[derive(Debug)]
pub struct Response {
}

#[derive(Debug)]
pub enum Action {
    Stat {
        resolve_links : bool,
        collect_ext_attrs : bool},
    Hash {
        max_size: u64,
        oversized_file_policy: HashActionOversizedFilePolicy,
        collect_ext_attrs: bool
    },
    Download {
        max_size: u64,
        oversized_file_policy: DownloadActionOversizedFilePolicy,
        use_external_stores: bool,
        collect_ext_attrs: bool,
        chunk_size: u64
    },
}

pub fn handle<S: Session>(session: &mut S, request: Request2) -> session::Result<()> {
    info!("Received request: {:?}", request);  // TODO: remove it

    // if request.pathtype.is_some() {
    //     Err("pathtype parameter is not supported!")
    // }
    //
    // if request.conditions.len() > 0{
    //     Err("conditions parameter is not supported!")
    // }
    //
    // if request.process_non_regular_files.is_some() {
    //     Err("process_non_regular_files parameter is not supported!")
    // }
    //
    // if request.follow_links.is_some() {
    //     Err("follow_links parameter is not supported!")
    // }
    //
    // if request.xdev.is_some() {
    //     Err("xdev parameter is not supported!")
    // }

    session.reply(Response {
    })?;
    Ok(())
}

trait ProtoEnum<T> {
    fn default() -> T;
    fn from_i32(val: i32) -> Option<T>;
}

impl ProtoEnum<ActionType> for ActionType {
    fn default() -> ActionType {
        FileFinderAction { ..Default::default() }.action_type()
    }
    fn from_i32(val: i32) -> Option<ActionType> {
        ActionType::from_i32(val)
    }
}

impl ProtoEnum<HashActionOversizedFilePolicy> for HashActionOversizedFilePolicy {
    fn default() -> HashActionOversizedFilePolicy {
        FileFinderHashActionOptions { ..Default::default() }.oversized_file_policy()
    }
    fn from_i32(val: i32) -> Option<HashActionOversizedFilePolicy> {
        HashActionOversizedFilePolicy::from_i32(val)
    }
}

impl ProtoEnum<DownloadActionOversizedFilePolicy> for DownloadActionOversizedFilePolicy {
    fn default() -> DownloadActionOversizedFilePolicy {
        FileFinderDownloadActionOptions { ..Default::default() }.oversized_file_policy()
    }
    fn from_i32(val: i32) -> Option<DownloadActionOversizedFilePolicy> {
        DownloadActionOversizedFilePolicy::from_i32(val)
    }
}

fn parse_enum<T : ProtoEnum<T>>(raw_enum_value: Option<i32>) -> Result<T, session::ParseError> {
    match raw_enum_value
    {
        Some(int_value) => match T::from_i32(int_value) {
            Some(parsed_value) => Ok(parsed_value),
            None => Err(session::ParseError::from(
                session::UnknownProtoEnumValue::new(
                    std::any::type_name::<T>(), int_value)))
        }
        None => Ok(T::default())
    }
}

impl super::Request for Request2 {
    type Proto = FileFinderArgs;

    fn from_proto(proto: FileFinderArgs) -> Result<Request2, session::ParseError> {
        let action: Option<Action> = match proto.action {
            Some(action) => Some({
                // FileFinderAction::action_type defines which action will be performed. Only
                // options from selected action are read.
                let action_type: ActionType =  parse_enum(action.action_type)?;
                match action_type {
                    ActionType::Stat =>
                    {
                    // TODO: move each (stat/hash/download) blocks to separate foos
                        let options : FileFinderStatActionOptions = action.stat.unwrap_or(
                            FileFinderStatActionOptions{..Default::default()});
                        Action::Stat {
                            resolve_links : options.resolve_links(),
                            collect_ext_attrs: options.collect_ext_attrs() }

                    },
                    ActionType::Hash => {
                        let options : FileFinderHashActionOptions = action.hash.unwrap_or(
                            FileFinderHashActionOptions{..Default::default()});
                        let oversized_file_policy: HashActionOversizedFilePolicy =
                            parse_enum(options.oversized_file_policy)?;
                        Action::Hash {
                            max_size : options.max_size(),
                            oversized_file_policy,
                            collect_ext_attrs : options.collect_ext_attrs()
                        }
                    },
                    ActionType::Download => {
                        let options : FileFinderDownloadActionOptions = action.download.unwrap_or(
                            FileFinderDownloadActionOptions{..Default::default()});
                        let oversized_file_policy: DownloadActionOversizedFilePolicy =
                            parse_enum(options.oversized_file_policy)?;
                        Action::Download {
                            max_size : options.max_size(),
                            oversized_file_policy,
                            collect_ext_attrs : options.collect_ext_attrs(),
                            use_external_stores: options.use_external_stores(),
                            chunk_size: options.chunk_size()
                        }
                    },
                }
            }),
            None => None
        };

        Ok(Request2 {
            paths: proto.paths,
            action
        })
    }
}

impl super::Response for Response {

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