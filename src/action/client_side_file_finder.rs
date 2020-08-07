// Copyright 2020 Google LLC
//
// Use of this source code is governed by an MIT-style license that can be found
// in the LICENSE file or at https://opensource.org/licenses/MIT.

//! TODO: add a comment

use crate::session::{self, Session, RegexParseError, UnknownEnumValueError};
use rrg_proto::{FileFinderArgs, FileFinderResult, Hash, FileFinderAction, FileFinderStatActionOptions, FileFinderHashActionOptions, FileFinderDownloadActionOptions, FileFinderCondition, FileFinderModificationTimeCondition, FileFinderAccessTimeCondition, FileFinderInodeChangeTimeCondition, FileFinderSizeCondition, FileFinderExtFlagsCondition, FileFinderContentsRegexMatchCondition, FileFinderContentsLiteralMatchCondition};
use log::info;
use std::convert::TryFrom;
use regex::Regex;

type ActionType = rrg_proto::file_finder_action::Action;
type HashActionOversizedFilePolicy = rrg_proto::file_finder_hash_action_options::OversizedFilePolicy;
type DownloadActionOversizedFilePolicy = rrg_proto::file_finder_download_action_options::OversizedFilePolicy;
type RegexMatchMode = rrg_proto::file_finder_contents_regex_match_condition::Mode;
type LiteralMatchMode = rrg_proto::file_finder_contents_literal_match_condition::Mode;
type ConditionType = rrg_proto::file_finder_condition::Type;
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
    ExtFlagsLinuxBitsSet(u32),
    ExtFlagsLinuxBitsUnset(u32),
    ExtFlagsOsxBitsSet(u32),
    ExtFlagsOsxBitsUnset(u32),
    ContentsRegexMatch(ContentsRegexMatchConditionOptions),
    ContentsLiteralMatch(ContentsLiteralMatchConditionOptions)
}

#[derive(Debug)]
pub struct ContentsRegexMatchConditionOptions {
    regex: Regex,
    mode: MatchMode,
    bytes_before: u32,
    bytes_after: u32,
    start_offset: u64,
    length: u64
}

#[derive(Debug)]
pub struct ContentsLiteralMatchConditionOptions {
    literal: Vec<u8>,
    mode: MatchMode,
    start_offset: u64,
    length: u64,
    bytes_before: u32,
    bytes_after: u32,
    xor_in_key: u32, // TODO: do not support for now
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
    fn default() -> Self { FileFinderHashActionOptions::default().oversized_file_policy() }
    fn from_i32(val: i32) -> Option<Self> { HashActionOversizedFilePolicy::from_i32(val) }
}

impl ProtoEnum<DownloadActionOversizedFilePolicy> for DownloadActionOversizedFilePolicy {
    fn default() -> Self { FileFinderDownloadActionOptions::default().oversized_file_policy() }
    fn from_i32(val: i32) -> Option<Self> { DownloadActionOversizedFilePolicy::from_i32(val) }
}

impl ProtoEnum<XDevMode> for XDevMode {
    fn default() -> Self {
        FileFinderArgs::default().xdev()
    }
    fn from_i32(val: i32) -> Option<Self> {
        XDevMode::from_i32(val)
    }
}

impl ProtoEnum<ConditionType> for ConditionType {
    fn default() -> Self {
        FileFinderCondition::default().condition_type()
    }
    fn from_i32(val: i32) -> Option<Self> {
        ConditionType::from_i32(val)
    }
}

impl ProtoEnum<RegexMatchMode> for RegexMatchMode {
    fn default() -> Self {
        FileFinderContentsRegexMatchCondition::default().mode()
    }
    fn from_i32(val: i32) -> Option<Self> {
        RegexMatchMode::from_i32(val)
    }
}

impl From<RegexMatchMode> for MatchMode
{
    fn from(proto: RegexMatchMode) -> Self {
        match proto {
            RegexMatchMode::FirstHit => MatchMode::FirstHit,
            RegexMatchMode::AllHits => MatchMode::AllHits
        }
    }
}

impl ProtoEnum<LiteralMatchMode> for LiteralMatchMode {
    fn default() -> Self {
        FileFinderContentsLiteralMatchCondition::default().mode()
    }
    fn from_i32(val: i32) -> Option<Self> {
        LiteralMatchMode::from_i32(val)
    }
}

impl From<LiteralMatchMode> for MatchMode
{
    fn from(proto: LiteralMatchMode) -> Self {
        match proto {
            LiteralMatchMode::FirstHit => MatchMode::FirstHit,
            LiteralMatchMode::AllHits => MatchMode::AllHits
        }
    }
}

fn parse_enum<T : ProtoEnum<T>>(raw_enum_value: Option<i32>) -> Result<T, session::ParseError> {
    match raw_enum_value
    {
        Some(int_value) => match T::from_i32(int_value) {
            Some(parsed_value) => Ok(parsed_value),
            None => Err(session::ParseError::from(  // TODO: remove ::from?
                session::UnknownEnumValueError::new(
                    std::any::type_name::<T>(), int_value)))
        }
        None => Ok(T::default())
    }
}

fn get_modification_time_conditions(proto : Option<FileFinderModificationTimeCondition>) -> Vec<Condition> {
    match proto {
        Some(options) => {
            let mut conditions: Vec<Condition> = vec![];
            if options.min_last_modified_time.is_some() {
                conditions.push(Condition::MinModificationTime(
                    time_from_micros(options.min_last_modified_time.unwrap())))
            }
            if options.max_last_modified_time.is_some() {
                conditions.push(Condition::MaxModificationTime(
                    time_from_micros(options.max_last_modified_time.unwrap())));
            }
            conditions
        }
        None => vec![]
    }
}

fn get_access_time_conditions(proto : Option<FileFinderAccessTimeCondition>) -> Vec<Condition> {
    match proto {
        Some(options) => {
            let mut conditions: Vec<Condition> = vec![];
            if options.min_last_access_time.is_some() {
                conditions.push(Condition::MinAccessTime(
                    time_from_micros(options.min_last_access_time.unwrap())))
            }
            if options.max_last_access_time.is_some() {
                conditions.push(Condition::MaxAccessTime(
                    time_from_micros(options.max_last_access_time.unwrap())));
            }
            conditions
        }
        None => vec![]
    }
}

fn get_inode_change_time_conditions(proto : Option<FileFinderInodeChangeTimeCondition>) -> Vec<Condition> {
    match proto {
        Some(options) => {
            let mut conditions: Vec<Condition> = vec![];
            if options.min_last_inode_change_time.is_some() {
                conditions.push(Condition::MinInodeChangeTime(
                    time_from_micros(options.min_last_inode_change_time.unwrap())))
            }
            if options.max_last_inode_change_time.is_some() {
                conditions.push(Condition::MaxInodeChangeTime(
                    time_from_micros(options.max_last_inode_change_time.unwrap())));
            }
            conditions
        }
        None => vec![]
    }
}

fn get_size_conditions(proto : Option<FileFinderSizeCondition>) -> Vec<Condition> {
    match proto {
        Some(options) => {
            let mut conditions: Vec<Condition> = vec![];
            if options.min_file_size.is_some() {
                conditions.push(Condition::MinInodeChangeTime(
                    time_from_micros(options.min_file_size.unwrap())))
            }
            conditions.push(Condition::MaxInodeChangeTime(
                time_from_micros(options.max_file_size())));
            conditions
        }
        None => vec![]
    }
}

fn get_ext_flags_condition(proto: Option<FileFinderExtFlagsCondition>) -> Vec<Condition> {
    match proto {
        Some(options) => {
            let mut conditions: Vec<Condition> = vec![];
            if options.linux_bits_set.is_some() {
                conditions.push(Condition::ExtFlagsLinuxBitsSet(
                    options.linux_bits_set.unwrap()));
            }
            if options.linux_bits_unset.is_some() {
                conditions.push(Condition::ExtFlagsLinuxBitsUnset(
                    options.linux_bits_unset.unwrap()));
            }
            if options.osx_bits_set.is_some() {
                conditions.push(Condition::ExtFlagsOsxBitsSet(
                    options.osx_bits_set.unwrap()));
            }
            if options.osx_bits_unset.is_some() {
                conditions.push(Condition::ExtFlagsOsxBitsUnset(
                    options.osx_bits_unset.unwrap()));
            }

            conditions
        }
        None => vec![]
    }
}

fn parse_regex(bytes : Vec<u8>) -> Result<Regex, session::RegexParseError> {
    let str = match std::str::from_utf8(bytes.as_slice())
    {
        Ok(v) => Ok(v),
        Err(e) => Err(RegexParseError::new(bytes.clone(), e.to_string()))
    }?;

    match Regex::new(str) {
        Ok(v) => Ok(v),
        Err(e) => Err(RegexParseError::new(bytes, e.to_string()))
    }
}

fn get_contents_regex_match_condition(proto : Option<FileFinderContentsRegexMatchCondition>) -> Result<Vec<Condition>, session::ParseError> {
    Ok(match proto {
        Some(options) => {
            let bytes_before = options.bytes_before();
            let bytes_after = options.bytes_after();
            let start_offset = options.start_offset();
            let length = options.length();
            let proto_mode : RegexMatchMode = parse_enum(options.mode)?;
            let mode= MatchMode::from(proto_mode);

            if options.regex.is_none() {
                return Ok(vec![])
            }
            let regex = parse_regex(options.regex.unwrap())?;

            let ret = ContentsRegexMatchConditionOptions {
                regex,
                mode,
                bytes_before,
                bytes_after,
                start_offset,
                length
            };
            vec![Condition::ContentsRegexMatch(ret)]
        }
        None => vec![]
    })
}

fn get_contents_literal_match_condition(proto : Option<FileFinderContentsLiteralMatchCondition>) -> Result<Vec<Condition>, session::ParseError> {
    Ok(match proto{
        Some(options) => {
            if options.literal.is_none() {
                return Ok(vec![])
            }

            let mode : LiteralMatchMode = parse_enum(options.mode)?;
            let ret = ContentsLiteralMatchConditionOptions {
                literal : options.literal.unwrap(),
                mode: MatchMode::from(mode),
                bytes_before: options.bytes_before(),
                bytes_after: options.bytes_after(),
                start_offset: options.start_offset(),
                length: options.length(),
                xor_in_key: options.xor_in_key(),
                xor_out_key: options.xor_out_key()
            };
            vec![Condition::ContentsLiteralMatch(ret)]
        }
        None => vec![]
    })
}

fn get_conditions(proto : FileFinderCondition) -> Result<Vec<Condition>, session::ParseError> {
    if proto.condition_type.is_none(){
        return Ok(vec![]);
    }
    let condition_type = parse_enum(proto.condition_type)?;

    Ok(match condition_type {
        ConditionType::ModificationTime => get_modification_time_conditions(proto.modification_time),
        ConditionType::AccessTime => get_access_time_conditions(proto.access_time),
        ConditionType::InodeChangeTime => get_inode_change_time_conditions(proto.inode_change_time),
        ConditionType::Size => get_size_conditions(proto.size),
        ConditionType::ExtFlags => get_ext_flags_condition(proto.ext_flags),
        ConditionType::ContentsRegexMatch => get_contents_regex_match_condition(proto.contents_regex_match)?,
        ConditionType::ContentsLiteralMatch => get_contents_literal_match_condition(proto.contents_literal_match)?
    })
}

impl super::Request for Request {
    type Proto = FileFinderArgs;

    fn from_proto(proto: FileFinderArgs) -> Result<Request, session::ParseError> {

        let follow_links = proto.follow_links();
        let process_non_regular_files = proto.process_non_regular_files();
        let xdev_mode = parse_enum(proto.xdev)?;

        let mut conditions = vec![];
        for proto_condition in proto.conditions {
            conditions.extend(get_conditions(proto_condition)?);
        }

        let action: Option<Action> = match proto.action {
            Some(action) => Some(Action::try_from(action)?),
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
// TODO: tests on a real FS