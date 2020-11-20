// MAYBE THIS FILE IS NOT NEEDED AT ALL!

use std::path::Path;
use crate::fs::Entry;
use log::warn;

#[derive(Debug, Clone, Copy)]
pub struct Options {
    // pub process_non_regular_files: bool,
    pub follow_links: bool,
    // pub xdev_mode: XDevMode,
}

pub fn list_dir<P: AsRef<Path>>(path: P, options: &Options) -> std::io::Result<ListDir> {
    let iter = std::fs::read_dir(path)?;

    Ok(ListDir {
        iter: iter,
        options: options.to_owned(),
    })
}

pub struct ListDir {
    iter: std::fs::ReadDir,
    options: Options,
}

impl std::iter::Iterator for ListDir {

    type Item = Entry;

    fn next(&mut self) -> Option<Entry> {
        for entry in &mut self.iter {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    warn!("directory iteration error: {}", error);
                    continue
                },
            };

            let path = entry.path();

            let stat = if self.options.follow_links {
                std::fs::metadata(&path)
            } else {
                std::fs::symlink_metadata(&path)
            };
            let metadata = match stat {
                Ok(metadata) => metadata,
                Err(error) => {
                    warn!("failed to stat '{}': {}", path.display(), error);
                    continue
                },
            };

            return Some(Entry {
                path: path,
                metadata: metadata,
            });
        }

        None
    }
}
