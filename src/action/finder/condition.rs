use crate::action::finder::request::Condition;
use rrg_proto::BufferReference;
use crate::fs::Entry;
use log::warn;
#[cfg(target_family = "unix")]
use std::os::unix::fs::MetadataExt;

pub struct ConditionResult {
    /// True if the condition was met.
    pub ok: bool,
    /// File locations matching the condition. Used only by ContentsRegexMatch
    /// and ContentsLiteralMatch conditions.
    pub matches: Vec<BufferReference>,  // TODO: use some better type passing data
}

impl ConditionResult {

    fn ok(ok: bool) -> ConditionResult{
        ConditionResult {
            ok,
            matches: vec![],
        }
    }
}

/// Checks is the condition is met by the entry.
/// In case of simple conditions if the data required for checking the condition
/// cannot be obtained then the condition is assumed to be met.
/// In case of content match conditions if the data can't be obtained then the
/// condition is assumed to not be met.
pub fn check_condition(condition: &Condition, entry: &Entry)  -> ConditionResult {
    match condition {
        Condition::MinModificationTime(expected) => {
            if let Ok(actual) = entry.metadata.modified(){
                ConditionResult::ok(actual >= *expected)
            }
            else {
                warn!("failed to obtain modification time for file: {}",
                entry.path.display());
                ConditionResult::ok(true)
            }
        }
        Condition::MaxModificationTime(expected) => {
            if let Ok(actual) = entry.metadata.modified(){
                ConditionResult::ok(actual <= *expected)
            }
            else {
                warn!("failed to obtain modification time for file: {}",
                      entry.path.display());
                ConditionResult::ok(true)
            }
        }
        Condition::MinAccessTime(expected) => {
            if let Ok(actual) = entry.metadata.accessed(){
                ConditionResult::ok(actual >= *expected)
            }
            else {
                warn!("failed to obtain access time for file: {}",
                      entry.path.display());
                ConditionResult::ok(true)
            }
        }
        Condition::MaxAccessTime(expected) => {
            if let Ok(actual) = entry.metadata.accessed(){
                ConditionResult::ok(actual <= *expected)
            }
            else {
                warn!("failed to obtain access time for file: {}",
                      entry.path.display());
                ConditionResult::ok(true)
            }
        }
        Condition::MinInodeChangeTime(expected) => {
            #[cfg(target_family = "unix")]
            if let Some(actual) = time_from_nanos(entry.metadata.ctime() as u64){
                return ConditionResult::ok(actual >= *expected);
            }
            else {
                warn!("failed to obtain inode change time for file: {}",
                      entry.path.display());
                return ConditionResult::ok(true);
            };

            ConditionResult::ok(true)
        }
        Condition::MaxInodeChangeTime(expected) => {
            #[cfg(target_family = "unix")]
            if let Some(actual) = time_from_nanos(entry.metadata.ctime() as u64){
                return ConditionResult::ok(actual <= *expected);
            }
            else {
                warn!("failed to obtain inode change time for file: {}",
                      entry.path.display());
                return ConditionResult::ok(true);
            };

            ConditionResult::ok(true)
        }
        Condition::MinSize(expected) => {
            ConditionResult::ok(entry.metadata.len() >= *expected)
        }
        Condition::MaxSize(expected) => {
            ConditionResult::ok(entry.metadata.len() <= *expected)
        }
        Condition::ExtFlagsLinuxBitsSet(expected) => {
            #[cfg(target_family = "unix")]

                ConditionResult::ok(true) // TODO: implement // reference: grr/core/grr_response_core/lib/util/filesystem.py  // extflags vs ext attributes
        }
        Condition::ExtFlagsLinuxBitsUnset(expected) => {
            ConditionResult::ok(true) // TODO: implement me
        }
        // Condition::ExtFlagsOsxBitsSet(expected) => {}
        // Condition::ExtFlagsOsxBitsUnset(expected) => {}
        // Condition::ContentsRegexMatch(_) => {}
        // Condition::ContentsLiteralMatch(_) => {}
    }
}

/// Coverts time from nanos (defined as nanoseconds from epoch
/// time: 1970-01-01T00:00:00.000000000Z) to `std::time::SystemTime`.
pub fn time_from_nanos(
    nanos: u64,
) -> Option<std::time::SystemTime> {
    std::time::UNIX_EPOCH
        .checked_add(std::time::Duration::from_nanos(nanos))
}
