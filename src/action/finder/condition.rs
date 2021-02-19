use crate::action::finder::file::{get_file_chunks, GetFileChunksConfig};
use crate::action::finder::request::{Condition, MatchMode, ContentsMatchCondition};
use crate::fs::linux::flags;
use crate::fs::Entry;
use log::warn;
use rrg_proto::BufferReference;
use std::cmp::{max, min};
#[cfg(target_family = "unix")]
use std::os::unix::fs::MetadataExt;

const BYTES_PER_CHUNK: usize = 10 * 1024 * 1024;
const OVERLAP_BYTES: usize = 1024 * 1024;

/// Returns true if all conditions were met.
pub fn check_conditions(
    conditions: &Vec<Condition>,
    entry: &Entry,
) -> bool {
    for condition in conditions {
        if !check_condition(condition, &entry){
            return false
        }
    }

    true
}

/// Returns positions of matches if and only if all `match_conditions` found
/// at least 1 match.
pub fn find_matches(
    match_conditions: &Vec<ContentsMatchCondition>,
    entry: &Entry,
) -> Vec<BufferReference> {
    let mut ret = vec![];
    for match_condition in match_conditions {
        let mut matches = matches(match_condition, &entry);
        if matches.is_empty() {
            return vec![];
        }
        ret.append(&mut matches);
    }

    ret
}

// TODO: update comment
/// Checks is the condition is met by the entry.
/// In case of simple conditions if the data required for checking the condition
/// cannot be obtained then the condition is assumed to be met.
/// In case of content match conditions if the data can't be obtained then the
/// condition is assumed to not be met.
fn check_condition(
    condition: &Condition,
    entry: &Entry,
) -> bool {
    match condition {
        Condition::ModificationTime{min, max} => {
            let mut ok = true;
            if let Ok(actual) = entry.metadata.modified() {
                if let Some(min) = min{
                    ok &= actual >= *min;
                }
                if let Some(max) = max {
                    ok &= actual <= *max;
                }
            } else {
                warn!(
                    "failed to obtain modification time for file: {}",
                    entry.path.display()
                );
            }

            ok
        }

        Condition::AccessTime{min, max} => {
            let mut ok = true;
            if let Ok(actual) = entry.metadata.accessed() {
                if let Some(min) = min{
                    ok &= actual >= *min;
                }
                if let Some(max) = max {
                    ok &= actual <= *max;
                }
            } else {
                warn!(
                    "failed to obtain access time for file: {}",
                    entry.path.display()
                );
            }

            ok
        }

        Condition::InodeChangeTime{min, max} => {
            let mut ok = true;

            #[cfg(target_family = "unix")]
            if let Some(actual) = time_from_nanos(entry.metadata.ctime() as u64)
            {
                if let Some(min) = min{
                    ok &= actual >= *min;
                }
                if let Some(max) = max{
                    ok &= actual <= *max;
                }
            } else {
                warn!(
                    "failed to obtain inode change time for file: {}",
                    entry.path.display()
                );
            };

            ok
        }

        Condition::Size{min, max} => {
            let mut ok = true;
            if let Some(min) = min {
                ok &= entry.metadata.len() >= *min;
            }

            if let Some(max) = max {
                ok &= entry.metadata.len() <= *max;
            }

            ok
        }

        Condition::ExtFlags{linux_bits_set, linux_bits_unset, ..} => {
            // TODO: support osx bits
            let mut ok = true;

            #[cfg(target_family = "unix")]
            if let Ok(flags) = flags(&entry.path) {
                if let Some(linux_bits_set) = linux_bits_set {
                    ok &= flags & linux_bits_set == flags;
                }
                if let Some(linux_bits_unset) = linux_bits_unset {
                    ok &= flags & linux_bits_unset == 0;
                }
            } else {
                warn!(
                    "failed to obtain extended flags for file: {}",
                    entry.path.display()
                );
            };

            ok
        }
    }
}

fn matches(condition: &ContentsMatchCondition, entry: &Entry) -> Vec<BufferReference> {
    let chunks = get_file_chunks(&entry.path, &GetFileChunksConfig{
        start_offset: condition.start_offset,
        max_read_bytes: condition.length,
        bytes_per_chunk: BYTES_PER_CHUNK,
        overlap_bytes: OVERLAP_BYTES,
    });
    let chunks = match chunks {
        Some(chunks) => chunks,
        None => return vec![],
    };

    let mut matches = vec![];
    let mut offset = 0;
    for chunk in chunks {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                warn!(
                    "failed to read chunk from file: {}, error: {}",
                    entry.path.display(),
                    err
                );
                return vec![];
            }
        };

        for m in condition.regex.find_iter(chunk.as_slice()) {
            let start =
                max(m.start() - condition.bytes_before as usize, 0);
            let end =
                min(m.end() + condition.bytes_after as usize, chunk.len());
            let data = chunk[start..end].to_vec();

            matches.push(BufferReference {
                offset: Some((offset + start) as u64),
                length: Some((end - start) as u64),
                callback: None,
                data: Some(data),
                pathspec: Some(entry.path.clone().into()),
            });

            match condition.mode {
                MatchMode::FirstHit => return matches,
                MatchMode::AllHits => (),
            }
        }
        offset += BYTES_PER_CHUNK - OVERLAP_BYTES;
    }

    matches
}

/// Coverts time from nanos (defined as nanoseconds from epoch
/// time: 1970-01-01T00:00:00.000000000Z) to `std::time::SystemTime`.
pub fn time_from_nanos(nanos: u64) -> Option<std::time::SystemTime> {
    std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_nanos(nanos))
}

// TODO: maybe split conditions to "stat_condition" (returning bool) and "match_condition" (returning vec<matches>)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Request as _;

    #[test]
    fn test_001() {
    }
}

// TODO: change regex condition to be a separate field in request
