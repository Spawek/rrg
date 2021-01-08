use std::path::{Component, Path, PathBuf};

/// Inlines `.` and `..` components in paths.
/// Returns unchanged input for non-absolute paths.
pub fn normalize(path: &Path) -> PathBuf {
    if !path.is_absolute() {
        return path.to_path_buf();
    }

    let mut components = vec![];
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(components.last(), Some(Component::RootDir)) {
                    components.pop();
                }
            }
            _ => {
                components.push(c);
            }
        }
    }

    components.iter().collect::<PathBuf>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_normalize_root() {
        let path = PathBuf::new().join(Component::RootDir);
        assert_eq!(normalize(&path), path);
    }

    #[test]
    fn test_normalize_relative_path() {
        let path = PathBuf::new()
            .join(Component::CurDir)
            .join(Component::ParentDir);
        assert_eq!(normalize(&path), path);
    }

    #[test]
    fn test_normalize_path_with_cur_dir() {
        let path = PathBuf::new()
            .join(Component::RootDir)
            .join(Component::Normal(&OsString::from("a")))
            .join(Component::CurDir)
            .join(Component::Normal(&OsString::from("b")));
        assert_eq!(
            normalize(&path),
            PathBuf::new()
                .join(Component::RootDir)
                .join(Component::Normal(&OsString::from("a")))
                .join(Component::Normal(&OsString::from("b")))
        );
    }

    #[test]
    fn test_normalize_path_with_parent_dir() {
        let path = PathBuf::new()
            .join(Component::RootDir)
            .join(Component::Normal(&OsString::from("a")))
            .join(Component::ParentDir)
            .join(Component::Normal(&OsString::from("b")));
        assert_eq!(
            normalize(&path),
            PathBuf::new()
                .join(Component::RootDir)
                .join(Component::Normal(&OsString::from("b")))
        );
    }

    #[test]
    fn test_normalize_path_with_parent_dirs_digging_below_root() {
        let path = PathBuf::new()
            .join(Component::RootDir)
            .join(Component::Normal(&OsString::from("a")))
            .join(Component::ParentDir)
            .join(Component::ParentDir)
            .join(Component::Normal(&OsString::from("b")));
        assert_eq!(
            normalize(&path),
            PathBuf::new()
                .join(Component::RootDir)
                .join(Component::Normal(&OsString::from("b")))
        );
    }
}
