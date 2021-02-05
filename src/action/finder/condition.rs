use crate::action::finder::chunks::{chunks, ChunksConfig};
use crate::action::finder::request::{Condition, MatchMode};
use crate::fs::linux::flags;
use crate::fs::Entry;
use log::warn;
use rrg_proto::{BufferReference, PathSpec};
use std::cmp::{max, min};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
#[cfg(target_family = "unix")]
use std::os::unix::fs::MetadataExt;

pub struct ConditionResult {
    /// True if the condition was met.
    pub ok: bool,
    /// File locations matching the condition. Used only by ContentsRegexMatch
    /// and ContentsLiteralMatch conditions.
    pub matches: Vec<BufferReference>, // TODO: use some better type passing data
}

impl ConditionResult {
    fn ok(ok: bool) -> ConditionResult {
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
pub fn check_condition(
    condition: &Condition,
    entry: &Entry,
) -> ConditionResult {
    match condition {
        Condition::MinModificationTime(expected) => {
            if let Ok(actual) = entry.metadata.modified() {
                ConditionResult::ok(actual >= *expected)
            } else {
                warn!(
                    "failed to obtain modification time for file: {}",
                    entry.path.display()
                );
                ConditionResult::ok(true)
            }
        }
        Condition::MaxModificationTime(expected) => {
            if let Ok(actual) = entry.metadata.modified() {
                ConditionResult::ok(actual <= *expected)
            } else {
                warn!(
                    "failed to obtain modification time for file: {}",
                    entry.path.display()
                );
                ConditionResult::ok(true)
            }
        }
        Condition::MinAccessTime(expected) => {
            if let Ok(actual) = entry.metadata.accessed() {
                ConditionResult::ok(actual >= *expected)
            } else {
                warn!(
                    "failed to obtain access time for file: {}",
                    entry.path.display()
                );
                ConditionResult::ok(true)
            }
        }
        Condition::MaxAccessTime(expected) => {
            if let Ok(actual) = entry.metadata.accessed() {
                ConditionResult::ok(actual <= *expected)
            } else {
                warn!(
                    "failed to obtain access time for file: {}",
                    entry.path.display()
                );
                ConditionResult::ok(true)
            }
        }
        Condition::MinInodeChangeTime(expected) => {
            let mut ret = true;

            #[cfg(target_family = "unix")]
            if let Some(actual) = time_from_nanos(entry.metadata.ctime() as u64)
            {
                ret = actual >= *expected;
            } else {
                warn!(
                    "failed to obtain inode change time for file: {}",
                    entry.path.display()
                );
            };

            ConditionResult::ok(ret)
        }
        Condition::MaxInodeChangeTime(expected) => {
            let mut ret = true;

            #[cfg(target_family = "unix")]
            if let Some(actual) = time_from_nanos(entry.metadata.ctime() as u64)
            {
                ret = actual <= *expected;
            } else {
                warn!(
                    "failed to obtain inode change time for file: {}",
                    entry.path.display()
                );
            };

            ConditionResult::ok(ret)
        }
        Condition::MinSize(expected) => {
            ConditionResult::ok(entry.metadata.len() >= *expected)
        }
        Condition::MaxSize(expected) => {
            ConditionResult::ok(entry.metadata.len() <= *expected)
        }
        Condition::ExtFlagsLinuxBitsSet(expected) => {
            let mut ret = true;

            #[cfg(target_family = "unix")]
            if let Ok(flags) = flags(&entry.path) {
                ret = flags & expected == flags;
            } else {
                warn!(
                    "failed to obtain extended flags for file: {}",
                    entry.path.display()
                );
            };

            ConditionResult::ok(ret)
        }
        Condition::ExtFlagsLinuxBitsUnset(expected) => {
            let mut ret = true;

            #[cfg(target_family = "unix")]
            if let Ok(flags) = flags(&entry.path) {
                ret = flags & expected == 0;
            } else {
                warn!(
                    "failed to obtain extended flags for file: {}",
                    entry.path.display()
                );
            };

            ConditionResult::ok(ret)
        }
        Condition::ExtFlagsOsxBitsSet(_) => {
            // TODO: not implemented
            ConditionResult::ok(true)
        }
        Condition::ExtFlagsOsxBitsUnset(_) => {
            // TODO: not implemented
            ConditionResult::ok(true)
        }
        Condition::ContentsRegexMatch(config) => {
            const BYTES_PER_CHUNK: usize = 10 * 1024 * 1024;
            const OVERLAP_BYTES: usize = 1024 * 1024;

            // TODO: separate foo
            let file = match File::open(&entry.path) {
                Ok(mut f) => {
                    if let Err(err) =
                        f.seek(SeekFrom::Start(config.start_offset as u64))
                    {
                        warn!(
                            "failed to seek in file: {}, error: {}",
                            entry.path.display(),
                            err
                        );
                        return ConditionResult::ok(false);
                    }
                    f.take(config.length)
                }
                Err(err) => {
                    warn!(
                        "failed to open file: {}, error: {}",
                        entry.path.display(),
                        err
                    );
                    return ConditionResult::ok(false);
                }
            };

            let reader = BufReader::new(file);
            let chunks = chunks(
                reader,
                ChunksConfig {
                    bytes_per_chunk: BYTES_PER_CHUNK,
                    overlap_bytes: OVERLAP_BYTES,
                },
            );

            let mut matches = vec![];
            let mut offset = 0;
            for chunk in chunks {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        warn!(
                            "failed to read file: {}, error: {}",
                            entry.path.display(),
                            err
                        );
                        return ConditionResult::ok(false);
                    }
                };

                for m in config.regex.find_iter(chunk.as_slice()) {
                    let start =
                        max(m.start() - config.bytes_before as usize, 0);
                    let end =
                        min(m.end() + config.bytes_after as usize, chunk.len());
                    let data = chunk[start..end].to_vec();

                    matches.push(BufferReference {
                        offset: Some((offset + start) as u64),
                        length: Some((end - start) as u64),
                        callback: None,
                        data: Some(data),
                        pathspec: Some(PathSpec {
                            // TODO: simplify this one
                            path: None,
                            ..Default::default()
                        }),
                    });

                    if matches!(config.mode, MatchMode::FirstHit) {
                        return ConditionResult { ok: true, matches };
                    }
                }
                offset += BYTES_PER_CHUNK - OVERLAP_BYTES;
            }

            ConditionResult {
                ok: !matches.is_empty(),
                matches,
            }
        }
        Condition::ContentsLiteralMatch(_) => ConditionResult::ok(true),
    }
}

/// Coverts time from nanos (defined as nanoseconds from epoch
/// time: 1970-01-01T00:00:00.000000000Z) to `std::time::SystemTime`.
pub fn time_from_nanos(nanos: u64) -> Option<std::time::SystemTime> {
    std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_nanos(nanos))
}

// TODO: maybe split conditions to "stat_condition" (returning bool) and "match_condition" (returning vec<matches>)
