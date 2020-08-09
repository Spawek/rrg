// Copyright 2020 Google LLC
//
// Use of this source code is governed by an MIT-style license that can be found
// in the LICENSE file or at https://opensource.org/licenses/MIT.

//! Defines an internal type for client side file finder action and provides a function converting
//! proto format of the request (rrg_proto::FileFinderArgs) to the internal format.

use regex::Regex;
use crate::session::{ParseError, RegexParseError, UnknownEnumValueError};
use rrg_proto::{FileFinderArgs, FileFinderAction, FileFinderStatActionOptions, FileFinderHashActionOptions, FileFinderDownloadActionOptions, FileFinderCondition, FileFinderModificationTimeCondition, FileFinderAccessTimeCondition, FileFinderInodeChangeTimeCondition, FileFinderSizeCondition, FileFinderExtFlagsCondition, FileFinderContentsRegexMatchCondition, FileFinderContentsLiteralMatchCondition};
use std::convert::TryFrom;

type HashActionOversizedFilePolicy = rrg_proto::file_finder_hash_action_options::OversizedFilePolicy;
type DownloadActionOversizedFilePolicy = rrg_proto::file_finder_download_action_options::OversizedFilePolicy;
type RegexMatchMode = rrg_proto::file_finder_contents_regex_match_condition::Mode;
type LiteralMatchMode = rrg_proto::file_finder_contents_literal_match_condition::Mode;
type ActionType = rrg_proto::file_finder_action::Action;
type ConditionType = rrg_proto::file_finder_condition::Type;
type XDevMode = rrg_proto::file_finder_args::XDev;
type PathType = rrg_proto::path_spec::PathType;

#[derive(Debug)]
pub struct Request {
    paths: Vec<String>,
    path_type: PathType,
    action: Option<Action>,
    conditions: Vec<Condition>,
    process_non_regular_files: bool,
    follow_links: bool,
    xdev_mode: XDevMode
}

#[derive(Debug)]
enum MatchMode{
    AllHits,
    FirstHit
}

#[derive(Debug)]
pub enum Action {
    Stat(StatActionOptions),
    Hash(HashActionOptions),
    Download(DownloadActionOptions)
}

#[derive(Debug)]
pub struct StatActionOptions {
    resolve_links : bool,
    collect_ext_attrs : bool
}

#[derive(Debug)]
pub struct HashActionOptions {
    max_size: u64,
    oversized_file_policy: HashActionOversizedFilePolicy,
    collect_ext_attrs: bool
}

#[derive(Debug)]
pub struct DownloadActionOptions {
    max_size: u64,
    oversized_file_policy: DownloadActionOversizedFilePolicy,
    use_external_stores: bool,
    collect_ext_attrs: bool,
    chunk_size: u64
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
    xor_in_key: u32,
    xor_out_key: u32
}

fn time_from_micros(micros : u64) -> std::time::SystemTime{
    return std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_micros(micros))
        .unwrap_or_else(||panic!("Cannot create std::time::SystemTime from micros: {}", micros));
}


impl TryFrom<rrg_proto::FileFinderAction> for Action {
    type Error = ParseError;

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
        Action::Stat(StatActionOptions {
            resolve_links: proto.resolve_links(),
            collect_ext_attrs: proto.collect_ext_attrs()
        })
    }
}

impl TryFrom<FileFinderHashActionOptions> for Action {
    type Error = ParseError;

    fn try_from(proto: FileFinderHashActionOptions) -> Result<Self, Self::Error>{
        let oversized_file_policy: HashActionOversizedFilePolicy =
            parse_enum(proto.oversized_file_policy)?;
        Ok(Action::Hash(HashActionOptions{
            oversized_file_policy,
            collect_ext_attrs : proto.collect_ext_attrs(),
            max_size: proto.max_size()
        }))
    }
}

impl TryFrom<FileFinderDownloadActionOptions> for Action {
    type Error = ParseError;

    fn try_from(proto: FileFinderDownloadActionOptions) -> Result<Self, Self::Error>{
        let oversized_file_policy: DownloadActionOversizedFilePolicy =
            parse_enum(proto.oversized_file_policy)?;
        Ok(Action::Download(DownloadActionOptions{
            oversized_file_policy,
            max_size : proto.max_size(),
            collect_ext_attrs : proto.collect_ext_attrs(),
            use_external_stores: proto.use_external_stores(),
            chunk_size: proto.chunk_size()
        }))
    }
}


trait ProtoEnum<T> {
    fn default() -> T;

    // Returns value of the enum or None if the input i32 does not describe any know enum value.
    fn from_i32(val: i32) -> Option<T>;
}

impl ProtoEnum<PathType> for PathType{
    fn default() -> PathType { FileFinderArgs::default().pathtype() }
    fn from_i32(val: i32) -> Option<PathType> { PathType::from_i32(val) }
}

impl ProtoEnum<ActionType> for ActionType {
    fn default() -> Self { FileFinderAction::default().action_type() }
    fn from_i32(val: i32) -> Option<Self> { ActionType::from_i32(val) }
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
    fn default() -> Self { FileFinderArgs::default().xdev() }
    fn from_i32(val: i32) -> Option<Self> { XDevMode::from_i32(val) }
}

impl ProtoEnum<ConditionType> for ConditionType {
    fn default() -> Self { FileFinderCondition::default().condition_type() }
    fn from_i32(val: i32) -> Option<Self> { ConditionType::from_i32(val) }
}

impl ProtoEnum<RegexMatchMode> for RegexMatchMode {
    fn default() -> Self { FileFinderContentsRegexMatchCondition::default().mode() }
    fn from_i32(val: i32) -> Option<Self> { RegexMatchMode::from_i32(val) }
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

fn parse_enum<T : ProtoEnum<T>>(raw_enum_value: Option<i32>) -> Result<T, ParseError> {
    match raw_enum_value
    {
        Some(int_value) => match T::from_i32(int_value) {
            Some(parsed_value) => Ok(parsed_value),
            None => Err(ParseError::from(
                UnknownEnumValueError::new(
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
                conditions.push(Condition::MinSize(options.min_file_size.unwrap()));
            }
            conditions.push(Condition::MaxSize(options.max_file_size()));
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

fn parse_regex(bytes : Vec<u8>) -> Result<Regex, RegexParseError> {
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

fn get_contents_regex_match_condition(proto : Option<FileFinderContentsRegexMatchCondition>) -> Result<Vec<Condition>, ParseError> {
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

fn get_contents_literal_match_condition(proto : Option<FileFinderContentsLiteralMatchCondition>) -> Result<Vec<Condition>, ParseError> {
    Ok(match proto{
        Some(options) => {
            let bytes_before = options.bytes_before();
            let bytes_after =options.bytes_after();
            let start_offset = options.start_offset();
            let length = options.length();
            let xor_in_key =options.xor_in_key();
            let xor_out_key = options.xor_out_key();
            let proto_mode : LiteralMatchMode = parse_enum(options.mode)?;
            let mode = MatchMode::from(proto_mode);

            if options.literal.is_none() {
                return Ok(vec![])
            }
            let literal = options.literal.unwrap();

            let ret = ContentsLiteralMatchConditionOptions {
                literal,
                mode,
                bytes_before,
                bytes_after,
                start_offset,
                length,
                xor_in_key,
                xor_out_key,
            };
            vec![Condition::ContentsLiteralMatch(ret)]
        }
        None => vec![]
    })
}

fn get_conditions(proto : FileFinderCondition) -> Result<Vec<Condition>, ParseError> {
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

impl super::super::Request for Request {
    type Proto = FileFinderArgs;

    fn from_proto(proto: FileFinderArgs) -> Result<Request, ParseError> {

        let follow_links = proto.follow_links();
        let process_non_regular_files = proto.process_non_regular_files();
        let xdev_mode = parse_enum(proto.xdev)?;
        let path_type = parse_enum(proto.pathtype)?;

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
            path_type,
            action,
            conditions,
            follow_links,
            process_non_regular_files,
            xdev_mode,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_request(args : FileFinderArgs) -> Request{
        let request : Result<Request, ParseError> =
            super::super::super::Request::from_proto(args);
        assert!(request.is_ok());
        request.unwrap()
    }

    fn get_parse_error(args : FileFinderArgs) -> ParseError{
        let request : Result<Request, ParseError> =
            super::super::super::Request::from_proto(args);
        assert!(request.is_err());
        request.unwrap_err()
    }

    #[test]
    fn empty_request_test() {
        let request = get_request(
            FileFinderArgs{..Default::default()});
        assert!(request.paths.is_empty());
        assert!(request.action.is_none());
        assert!(request.conditions.is_empty());
        assert_eq!(request.path_type, PathType::Os);
        assert_eq!(request.process_non_regular_files, false);
        assert_eq!(request.follow_links, false);
        assert_eq!(request.xdev_mode, XDevMode::Local);
    }

    #[test]
    fn basic_root_parameters_test() {
        let request = get_request(
            FileFinderArgs{
            paths: vec!["abc".to_string(), "cba".to_string()],
            pathtype: Some(PathType::Registry as i32),
            process_non_regular_files: Some(true),
            follow_links: Some(true),
            xdev: Some(rrg_proto::file_finder_args::XDev::Always as i32),
            ..Default::default()});
        assert_eq!(request.paths, vec!["abc".to_string(), "cba".to_string()]);
        assert!(request.action.is_none());
        assert_eq!(request.path_type, PathType::Registry);
        assert_eq!(request.process_non_regular_files, true);
        assert_eq!(request.follow_links, true);
        assert_eq!(request.xdev_mode, XDevMode::Always);
    }

    #[test]
    fn default_stats_action_test() {
        let request = get_request(
            FileFinderArgs{
            action: Some(
                FileFinderAction{
                    action_type: Some(ActionType::Stat as i32),
                    ..Default::default()}),
                 ..Default::default()});
        assert!(request.action.is_some());
        match request.action.unwrap() {
            Action::Stat(options) => {
                assert_eq!(options.collect_ext_attrs, false);
                assert_eq!(options.resolve_links, false);
            }
            _ => panic!("Unexpected action type")
        }
    }

    #[test]
    fn stats_action_test() {
        let request = get_request(
             FileFinderArgs{
            action: Some(
                FileFinderAction{
                    action_type: Some(ActionType::Stat as i32),
                    stat: Some(FileFinderStatActionOptions{
                        resolve_links: Some(true),
                        collect_ext_attrs: Some(true)
                    }),
                    ..Default::default()}),
            ..Default::default()});
        assert!(request.action.is_some());
        match request.action.unwrap() {
            Action::Stat(options) => {
                assert_eq!(options.collect_ext_attrs, true);
                assert_eq!(options.resolve_links, true);
            }
            _ => panic!("Unexpected action type")
        }
    }

    #[test]
    fn default_hash_action_test() {
        let request = get_request(
             FileFinderArgs{
            action: Some(
                FileFinderAction{
                    action_type: Some(ActionType::Hash as i32),
                    ..Default::default()}),
            ..Default::default()});
        assert!(request.action.is_some());
        match request.action.unwrap() {
            Action::Hash(options) => {
                assert_eq!(options.collect_ext_attrs, false);
                assert_eq!(options.oversized_file_policy, HashActionOversizedFilePolicy::Skip);
                assert_eq!(options.max_size, 500000000);
            }
            _ => panic!("Unexpected action type")
        }
    }

    #[test]
    fn hash_action_test() {
        let request = get_request(
             FileFinderArgs{
            action: Some(
                FileFinderAction{
                    action_type: Some(ActionType::Hash as i32),
                    hash: Some(FileFinderHashActionOptions{
                        collect_ext_attrs: Some(true),
                        oversized_file_policy: Some(HashActionOversizedFilePolicy::HashTruncated as i32),
                        max_size: Some(123456),
                    }),
                    ..Default::default()}),
            ..Default::default()});
        assert!(request.action.is_some());
        match request.action.unwrap() {
            Action::Hash(options) => {
                assert_eq!(options.collect_ext_attrs, true);
                assert_eq!(options.oversized_file_policy, HashActionOversizedFilePolicy::HashTruncated);
                assert_eq!(options.max_size, 123456);
            }
            _ => panic!("Unexpected action type")
        }
    }

    #[test]
    fn default_download_action_test() {
        let request = get_request(
            FileFinderArgs{
            action: Some(
                FileFinderAction{
                    action_type: Some(ActionType::Download as i32),
                    ..Default::default()}),
            ..Default::default()});
        assert!(request.action.is_some());
        match request.action.unwrap() {
            Action::Download(options) => {
                assert_eq!(options.max_size, 500000000);
                assert_eq!(options.oversized_file_policy, DownloadActionOversizedFilePolicy::Skip);
                assert_eq!(options.use_external_stores, true);
                assert_eq!(options.collect_ext_attrs, false);
                assert_eq!(options.chunk_size, 524288);
            }
            _ => panic!("Unexpected action type")
        }
    }

    #[test]
    fn download_action_test() {
        let request = get_request(
            FileFinderArgs{
            action: Some(
                FileFinderAction{
                    action_type: Some(ActionType::Download as i32),
                    download: Some(FileFinderDownloadActionOptions{
                        max_size: Some(12345),
                        collect_ext_attrs: Some(true),
                        oversized_file_policy: Some(DownloadActionOversizedFilePolicy::DownloadTruncated as i32),
                        use_external_stores: Some(false),
                        chunk_size: Some(5432)
                    }),
                    ..Default::default()}),
            ..Default::default()});
        assert!(request.action.is_some());
        match request.action.unwrap() {
            Action::Download(options) => {
                assert_eq!(options.max_size, 12345);
                assert_eq!(options.oversized_file_policy, DownloadActionOversizedFilePolicy::DownloadTruncated);
                assert_eq!(options.use_external_stores, false);
                assert_eq!(options.collect_ext_attrs, true);
                assert_eq!(options.chunk_size, 5432);
            }

            _ => panic!("Unexpected action type")
        }
    }

    #[test]
    fn error_on_parsing_unknown_enum_value() {
        let err = get_parse_error(FileFinderArgs{
            action: Some(
                FileFinderAction{
                    action_type: Some(345 as i32),
                    ..Default::default()
                    }),
                ..Default::default()});
        match err{
            ParseError::UnknownEnumValue(error) => {
                assert_eq!(error.enum_name, std::any::type_name::<ActionType>());
                assert_eq!(error.value, 345);
            }
            _ => panic!("Unexpected error type")
        }
    }

}