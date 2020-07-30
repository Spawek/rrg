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
use std::convert::TryFrom;

type ActionType = rrg_proto::file_finder_action::Action;
type HashActionOversizedFilePolicy = rrg_proto::file_finder_hash_action_options::OversizedFilePolicy;
type DownloadActionOversizedFilePolicy = rrg_proto::file_finder_download_action_options::OversizedFilePolicy;
type RegexMatchMode = rrg_proto::file_finder_contents_regex_match_condition::Mode;
type LiteralMatchMode = rrg_proto::file_finder_contents_literal_match_condition::Mode;
type XDevMode = rrg_proto::file_finder_args::XDev;

#[derive(Debug)]
enum MatchMode{
    AllHits,
    FirstHit
}

#[derive(Debug)]
pub struct Request {
    paths: Vec<String>,
    action: Option<Action>,
    conditions: Vec<Condition>,
    process_non_regular_files: bool,
    follow_links: bool,
    xdev_mode: XDevMode
}

#[derive(Debug)]
pub struct Response {
}

#[derive(Debug)]
pub enum Action {
    Stat {
        resolve_links : bool,
        collect_ext_attrs : bool  // TODO: move outside (as it's in all versions?)
    },
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

#[derive(Debug)]
pub enum Condition {
    MinModificationTime(std::time::SystemTime),
    MaxModificationTime(std::time::SystemTime),
    MinAccessTime(std::time::SystemTime),
    MaxAccessTime(std::time::SystemTime),
    MinInodeChangeTime(std::time::SystemTime),
    MaxInodeChangeTime(std::time::SystemTime),
    MinSize(u64),
    MaxSize(u64),
    ExtFlags(ExtFlagsCondition),
    ContentsRegexMatch(ContentsRegexMatchCondition),
    ContentsLiteralMatch(ContentsLiteralMatchCondition)
}

#[derive(Debug)]
pub enum ExtFlagsCondition {
    LinuxBitsSet(u32),
    LinuxBitsUnset(u32),
    OsxBitsSet(u32),
    OsxBitsUnset(u32)
}

#[derive(Debug)]
pub struct ContentsRegexMatchCondition {
    regex : String, // TODO: change to some Vec<u8> type
    mode: MatchMode,
    bytes_before: u32,
    bytes_after: u32,
    start_offset: u64,
    length: u64
}

#[derive(Debug)]
pub struct ContentsLiteralMatchCondition {
    literal : String, // TODO: change to some Vec<u8> type
    mode: MatchMode,
    start_offset: u64,
    length: u64,
    bytes_before: u32,
    bytes_after: u32,
    xor_in_key: u32,
    xor_out_key: u32
}

pub fn time_from_micros(micros : u64) -> std::time::SystemTime{
    return std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_micros(micros))
        .unwrap_or_else(||panic!("Cannot create std::time::SystemTime from micros: {}", micros));
}

impl TryFrom<rrg_proto::FileFinderAction> for Action {
    type Error = session::ParseError;

    fn try_from(proto: rrg_proto::FileFinderAction) -> Result<Self, Self::Error> {
        // FileFinderAction::action_type defines which action will be performed. Only
        // options from selected action are read.
        let action_type: ActionType =  parse_enum(proto.action_type)?;
        Ok(match action_type {
            ActionType::Stat => Action::from(proto.stat.unwrap_or_default()),
            ActionType::Hash => Action::try_from(proto.hash.unwrap_or_default())?,
            ActionType::Download => Action::try_from(proto.download.unwrap_or_default())?
        })
    }
}

impl From<FileFinderStatActionOptions> for Action {
    fn from(proto: FileFinderStatActionOptions) -> Action {
        Action::Stat {
            resolve_links : proto.resolve_links(),
            collect_ext_attrs: proto.collect_ext_attrs()
        }
    }
}

impl TryFrom<FileFinderHashActionOptions> for Action {
    type Error = session::ParseError;

    fn try_from(proto: FileFinderHashActionOptions) -> Result<Self, Self::Error>{
        let oversized_file_policy: HashActionOversizedFilePolicy =
            parse_enum(proto.oversized_file_policy)?;
        Ok(Action::Hash {
            oversized_file_policy,
            collect_ext_attrs : proto.collect_ext_attrs(),
            max_size: proto.max_size()
        })
    }
}

impl TryFrom<FileFinderDownloadActionOptions> for Action {
    type Error = session::ParseError;

    fn try_from(proto: FileFinderDownloadActionOptions) -> Result<Self, Self::Error>{
        let oversized_file_policy: DownloadActionOversizedFilePolicy =
            parse_enum(proto.oversized_file_policy)?;
        Ok(Action::Download {
            oversized_file_policy,
            max_size : proto.max_size(),
            collect_ext_attrs : proto.collect_ext_attrs(),
            use_external_stores: proto.use_external_stores(),
            chunk_size: proto.chunk_size()
        })
    }
}

pub fn handle<S: Session>(session: &mut S, request: Request) -> session::Result<()> {
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

    // Returns value of the enum or None if the input i32 does not describe any know enum value.
    fn from_i32(val: i32) -> Option<T>;
}

impl ProtoEnum<ActionType> for ActionType {
    fn default() -> Self {
        FileFinderAction::default().action_type()
    }
    fn from_i32(val: i32) -> Option<Self> {
        ActionType::from_i32(val)
    }
}

impl ProtoEnum<HashActionOversizedFilePolicy> for HashActionOversizedFilePolicy {
    fn default() -> Self {
        FileFinderHashActionOptions::default().oversized_file_policy()
    }
    fn from_i32(val: i32) -> Option<Self> {
        HashActionOversizedFilePolicy::from_i32(val)
    }
}

impl ProtoEnum<DownloadActionOversizedFilePolicy> for DownloadActionOversizedFilePolicy {
    fn default() -> Self {
        FileFinderDownloadActionOptions::default().oversized_file_policy()
    }
    fn from_i32(val: i32) -> Option<Self> {
        DownloadActionOversizedFilePolicy::from_i32(val)
    }
}

impl ProtoEnum<XDevMode> for XDevMode {
    fn default() -> Self {
        FileFinderArgs::default().xdev()
    }
    fn from_i32(val: i32) -> Option<Self> {
        XDevMode::from_i32(val)
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

impl super::Request for Request {
    type Proto = FileFinderArgs;

    fn from_proto(proto: FileFinderArgs) -> Result<Request, session::ParseError> {

        let follow_links = proto.follow_links();
        let process_non_regular_files = proto.process_non_regular_files();
        let xdev_mode = parse_enum(proto.xdev)?;
        
        let conditions = vec![]; // TODO: implement me!

        // TODO: can I make this statement look better?
        let action: Option<Action> = match proto.action {
            Some(action) => Some(Action::try_from(action)?),  // TODO: this '?' may be confusing
            None => None
        };

        Ok(Request {
            paths: proto.paths,
            action,
            conditions,
            follow_links,
            process_non_regular_files,
            xdev_mode,
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