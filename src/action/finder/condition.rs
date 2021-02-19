use crate::action::finder::file::{get_file_chunks, GetFileChunksConfig};
use crate::action::finder::request::{
    Condition, ContentsMatchCondition, MatchMode,
};
use crate::fs::linux::flags;
use crate::fs::Entry;
use log::warn;
use rrg_macro::ack;
use rrg_proto::BufferReference;
use std::cmp::{max, min};
#[cfg(target_family = "unix")]
use std::os::unix::fs::MetadataExt;

const BYTES_PER_CHUNK: usize = 10 * 1024 * 1024;
const OVERLAP_BYTES: usize = 1024 * 1024;

/// Returns true if all conditions were met.
/// If the data required for checking the condition cannot be obtained then
/// the condition is assumed to be met.
pub fn check_conditions(conditions: &Vec<Condition>, entry: &Entry) -> bool {
    conditions.into_iter().all(|c| check_condition(c, &entry))
}

/// Returns positions of matches from all conditions when all
/// `match_conditions` found at least 1 match. Returns empty `Vec` otherwise.
/// If the file content cannot be obtained the condition is assumed to
/// be not met.
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

/// Checks is the condition is met by the entry.
/// In case of simple conditions if the data required for checking the condition
/// cannot be obtained then the condition is assumed to be met.
fn check_condition(condition: &Condition, entry: &Entry) -> bool {
    match condition {
        Condition::ModificationTime { min, max } => {
            let actual = ack! {
                entry.metadata.modified(),
                error: "failed to obtain modification time"
            };
            match actual {
                Some(actual) => is_in_range(&actual, (min, max)),
                None => true,
            }
        }

        Condition::AccessTime { min, max } => {
            let actual = ack! {
                entry.metadata.accessed(),
                error: "failed to obtain access time";
            };
            match actual {
                Some(actual) => is_in_range(&actual, (min, max)),
                None => true,
            }
        }

        Condition::InodeChangeTime { min, max } => {
            match time_from_nanos(entry.metadata.ctime() as u64) {
                Some(actual) => is_in_range(&actual, (min, max)),
                None => {
                    warn!(
                        "failed to obtain inode change time for file: {}",
                        entry.path.display()
                    );
                    true
                }
            }
        }

        Condition::Size { min, max } => {
            is_in_range(&entry.metadata.len(), (min, max))
        }

        Condition::ExtFlags {
            linux_bits_set,
            linux_bits_unset,
            ..
        } => {
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

/// Checks if `value` is in range [`min`, `max`] (inclusive on both ends).
/// If range option is equal `None` then the condition is not checked.
fn is_in_range<T: Ord>(value: &T, range: (&Option<T>, &Option<T>)) -> bool {
    if let Some(min) = &range.0 {
        if value < min {
            return false;
        }
    }
    if let Some(max) = &range.1 {
        if value > max {
            return false;
        }
    }

    true
}

fn matches(
    condition: &ContentsMatchCondition,
    entry: &Entry,
) -> Vec<BufferReference> {
    let chunks = get_file_chunks(
        &entry.path,
        &GetFileChunksConfig {
            start_offset: condition.start_offset,
            max_read_bytes: condition.length,
            bytes_per_chunk: BYTES_PER_CHUNK,
            overlap_bytes: OVERLAP_BYTES,
        },
    );
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
            let start = max(m.start() - condition.bytes_before as usize, 0);
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

    #[test]
    fn test_001() {}
}

// TODO: change regex condition to be a separate field in request
