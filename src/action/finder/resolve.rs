use std::path::Path;
use crate::fs::{Entry, ListDir, list_dir};
use crate::action::finder::task::{build_task, Task, PathComponent, TaskBuilder};
use crate::action::finder::error::Error;
use crate::action::finder::path::normalize;
use regex::Regex;
use log::warn;

pub fn resolve_path(
    path: &Path,
    follow_links: bool,
) -> Result<impl Iterator<Item=Entry>, Error> {
    let task = build_task(path)?;
    Ok(ResolvePath {
        outputs: vec![],
        tasks: vec![task],
        follow_links,
    })
}


/// Implements `Iterator` for resolving all entries in a path, which can
/// contain globs (e.g. '123*') or recursive scans (e.g. '**').
struct ResolvePath {
    /// Results buffered to be returned.
    outputs: Vec<Entry>,
    /// Remaining tasks to be executed.
    tasks: Vec<Task>,
    /// If true then symbolic links should be followed in recursive scans.
    follow_links: bool,
}

fn normalize_path(e: Entry) -> Entry {
    Entry {
        metadata: e.metadata,
        path: normalize(&e.path),
    }
}

impl std::iter::Iterator for ResolvePath {
    type Item = Entry;

    fn next(&mut self) -> Option<Entry> {
        loop {
            if let Some(v) = self.outputs.pop() {
                return Some(v);
            }

            let task = self.tasks.pop()?;
            let mut task_results = resolve_task(task, self.follow_links);
            self.tasks.append(&mut task_results.new_tasks);
            let outputs = task_results.outputs.into_iter().map(normalize_path);
            self.outputs.extend(outputs);
        }
    }
}

/// Routes resolving task to one of the subfunctions.
fn resolve_task(task: Task, follow_links: bool) -> TaskResults {
    match &task.current_component {
        PathComponent::Constant(path) => resolve_constant_task(path),
        PathComponent::Glob(regex) => resolve_glob_task(
            regex,
            &task.path_prefix,
            &task.remaining_components,
        ),
        PathComponent::RecursiveScan { max_depth } => {
            resolve_recursive_scan_task(
                *max_depth,
                &task.path_prefix,
                &task.remaining_components,
                follow_links,
            )
        }
    }
}

/// Implements `Iterator` for getting all entries in a given path.`
enum ListPath {
    Next(Option<Entry>),
    ListDir(ListDir),
}

impl std::iter::Iterator for ListPath {
    type Item = Entry;

    fn next(&mut self) -> Option<Entry> {
        match self {
            ListPath::Next(next) => next.take(),
            ListPath::ListDir(iter) => iter.next(),
        }
    }
}

/// Returns iterator over all entries in a given path.
fn list_path(path: &Path) -> impl Iterator<Item=Entry> {
    let metadata = match path.metadata() {
        Ok(v) => v,
        Err(err) => {
            warn!("failed to stat '{}': {}", path.display(), err);
            return ListPath::Next(None);
        }
    };

    if !metadata.is_dir() {
        ListPath::Next(Some(Entry {
            path: path.to_owned(),
            metadata,
        }));
    }

    match list_dir(path) {
        Ok(v) => ListPath::ListDir(v),
        Err(err) => {
            warn!("listing directory '{}' failed :{}", path.display(), err);
            ListPath::Next(None)
        }
    }
}

#[derive(Debug)]
struct TaskResults {
    new_tasks: Vec<Task>,
    outputs: Vec<Entry>,
}

/// Returns true if last component `path` matches `regex`.
/// E.g. '/home/abc/111' matches '[1]*' regex.
fn last_component_matches(path: &Path, regex: &Regex) -> bool {
    let last_component = match path.components().last() {
        Some(v) => v,
        None => {
            warn!(
                "failed to fetch last component from path: {}",
                path.display()
            );
            return false;
        }
    };

    let last_component = match last_component.as_os_str().to_str() {
        Some(v) => v,
        None => {
            warn!(
                "failed to convert last component of the path to string: {}",
                path.display()
            );
            return false;
        }
    };

    regex.is_match(last_component)
}

/// Resolves glob expression (e.g. '123*') in path.
fn resolve_glob_task(
    glob: &Regex,
    path_prefix: &Path,
    remaining_components: &Vec<PathComponent>,
) -> TaskResults {
    let mut new_tasks = vec![];
    let mut outputs = vec![];
    for e in list_path(&path_prefix) {
        if last_component_matches(&e.path, &glob) {
            if remaining_components.is_empty() {
                outputs.push(e.clone());
            } else {
                let new_task = TaskBuilder::new()
                    .add_constant(&e.path)
                    .add_components(remaining_components.clone())
                    .build();
                new_tasks.push(new_task);
            }
        }
    }

    TaskResults { new_tasks, outputs }
}

/// Checks if Entry is a directory using `metadata` if `follow_links` is set
/// or `symlink_metadata` otherwise.
fn is_dir(e: &Entry, follow_links: bool) -> bool {
    if e.metadata.is_dir() {
        return true;
    }

    if follow_links {
        match std::fs::metadata(&e.path) {
            Ok(metadata) => {
                return metadata.is_dir();
            }
            Err(err) => {
                warn!("failed to stat '{}': {}", e.path.display(), err);
                return false;
            }
        }
    }

    return false;
}

/// Resolves recursive expression (e.g. '**') in path.
fn resolve_recursive_scan_task(
    max_depth: i32,
    path_prefix: &Path,
    remaining_components: &Vec<PathComponent>,
    follow_links: bool,
) -> TaskResults {
    let mut new_tasks = vec![];
    let mut outputs = vec![];
    for e in list_path(&path_prefix) {
        if !is_dir(&e, follow_links) {
            if remaining_components.is_empty() {
                outputs.push(e.to_owned());
            }
            continue;
        }

        let subdir_scan = TaskBuilder::new()
            .add_constant(&e.path)
            .add_components(remaining_components.clone())
            .build();
        new_tasks.push(subdir_scan);

        if max_depth > 1 {
            let mut recursive_scan = TaskBuilder::new().add_constant(&e.path);
            recursive_scan = recursive_scan.add_recursive_scan(max_depth - 1);
            recursive_scan =
                recursive_scan.add_components(remaining_components.clone());
            new_tasks.push(recursive_scan.build());
        }
    }

    TaskResults { new_tasks, outputs }
}

/// Resolves constant expression (just a plain name) in path.
fn resolve_constant_task(path: &Path) -> TaskResults {
    let mut ret = TaskResults {
        new_tasks: vec![],
        outputs: vec![],
    };

    if !path.exists() {
        return ret;
    }

    let metadata = match path.metadata() {
        Ok(v) => v,
        Err(err) => {
            warn!("failed to stat '{}': {}", path.display(), err);
            return ret;
        }
    };

    ret.outputs.push(Entry {
        path: path.to_owned(),
        metadata,
    });

    ret
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_path_with_file() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::write(tempdir.path().join("a"), "").unwrap();

        let request = tempdir.path().join("a");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, request);
        assert!(resolved[0].metadata.is_file());
    }

    #[test]
    fn test_constant_path_with_empty_dir() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("a")).unwrap();

        let request = tempdir.path();
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, request.to_path_buf());
        assert!(resolved[0].metadata.is_dir());
    }

    #[test]
    fn test_constant_path_with_nonempty_dir() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("a");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, request);
    }

    #[test]
    fn test_constant_path_when_file_doesnt_exist() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();

        let request = tempdir.path().join("abc");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 0);
    }

    #[test]
    fn test_constant_path_containing_parent_directory() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("a")).unwrap();
        std::fs::create_dir(tempdir.path().join("a").join("b")).unwrap();
        std::fs::create_dir(tempdir.path().join("a").join("c")).unwrap();

        let request = tempdir.path().join("a").join("b").join("..").join("c");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("a").join("c"));
    }

    #[test]
    fn test_glob_star() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abbc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abbd")).unwrap();
        std::fs::create_dir(tempdir.path().join("xbbc")).unwrap();

        let request = tempdir.path().join("a*c");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abbc"));
    }

    #[test]
    fn test_glob_star_doesnt_return_intermediate_directories() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b").join("c");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("*").join("*");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("a").join("b"));
    }

    #[test]
    fn test_glob_star_followed_by_constant() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("abc").join("123").join("qwe");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("*").join("123");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abc").join("123"));
    }

    #[test]
    fn test_glob_selection() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("xbc")).unwrap();

        let request = tempdir.path().join("ab[c]");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abc"));
    }

    #[test]
    fn test_glob_reverse_selection() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("abe")).unwrap();

        let request = tempdir.path().join("ab[!de]");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abc"));
    }

    #[test]
    fn test_glob_wildcard() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        std::fs::create_dir(tempdir.path().join("abc")).unwrap();
        std::fs::create_dir(tempdir.path().join("abd")).unwrap();
        std::fs::create_dir(tempdir.path().join("abe")).unwrap();
        std::fs::create_dir(tempdir.path().join("ac")).unwrap();

        let request = tempdir.path().join("a?c");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("abc"));
    }

    #[test]
    fn test_glob_recurse_default_max_depth() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b").join("c").join("d");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("**").join("c");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].path,
            tempdir.path().join("a").join("b").join("c")
        );
    }

    #[test]
    fn test_glob_recurse_too_low_max_depth_limit() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b").join("c");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("**1").join("c");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 0);
    }

    #[test]
    fn test_glob_recurse_at_the_end_of_the_path() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let a = tempdir.path().join("a");
        std::fs::create_dir(&a).unwrap();
        let file = a.join("file");
        std::fs::write(&file, "").unwrap();

        let request = tempdir.path().join("**");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 2);
        assert!(resolved.iter().find(|x| x.path == a).is_some());
        assert!(resolved.iter().find(|x| x.path == file).is_some());
    }

    #[test]
    fn test_glob_recurse_max_depth() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b").join("c").join("d");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("**2").join("c");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].path,
            tempdir.path().join("a").join("b").join("c")
        );
    }

    #[test]
    fn test_glob_recurse_and_parent_component_in_path() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("**").join("..").join("b");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, tempdir.path().join("a").join("b"));
    }

    #[test]
    fn test_directory_name_containing_glob_characters() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("a").join("b*[xyz]").join("c");
        std::fs::create_dir_all(path).unwrap();

        let request = tempdir.path().join("a").join("*").join("c");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].path,
            tempdir.path().join("a").join("b*[xyz]").join("c")
        );
    }

    #[test]
    #[cfg(target_family = "unix")]
    fn test_resolve_link_in_const_path() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();

        let a = tempdir.path().join("a");
        std::fs::create_dir(&a).unwrap();
        std::fs::write(a.join("file"), "").unwrap();

        let symlink = tempdir.path().join("b");
        std::os::unix::fs::symlink(&a, &symlink).unwrap();

        {
            let request = symlink.to_owned();
            let resolved = resolve_path(&request, follow_links)
                .unwrap()
                .collect::<Vec<_>>();
            assert_eq!(resolved.len(), 1);
            assert_eq!(resolved[0].path, symlink);
        }

        {
            let request = symlink.join("file").to_owned();
            let resolved = resolve_path(&request, follow_links)
                .unwrap()
                .collect::<Vec<_>>();
            assert_eq!(resolved.len(), 1);
            assert_eq!(resolved[0].path, symlink.join("file"));
        }
    }

    #[test]
    #[cfg(target_family = "unix")]
    fn test_resolve_link_in_glob() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();

        let a = tempdir.path().join("a");
        std::fs::create_dir(&a).unwrap();
        std::fs::write(a.join("file"), "").unwrap();

        let b = tempdir.path().join("b");
        std::fs::create_dir(&b).unwrap();
        let symlink = b.join("link_to_a");
        std::os::unix::fs::symlink(&a, &symlink).unwrap();

        let request = tempdir.path().join("b").join("*").join("file");

        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, symlink.join("file"));
    }

    #[test]
    #[cfg(target_family = "unix")]
    fn test_resolve_link_in_recursive_search_with_no_follow() {
        let follow_links = false;
        let tempdir = tempfile::tempdir().unwrap();

        let a = tempdir.path().join("a");
        std::fs::create_dir(&a).unwrap();
        std::fs::write(a.join("file"), "").unwrap();

        let b = tempdir.path().join("b");
        std::fs::create_dir(&b).unwrap();
        let symlink = b.join("link_to_a");
        std::os::unix::fs::symlink(&a, &symlink).unwrap();

        let request = b.join("**");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].path, symlink);
    }

    #[test]
    #[cfg(target_family = "unix")]
    fn test_resolve_link_in_recursive_search_with_follow() {
        let follow_links = true;
        let tempdir = tempfile::tempdir().unwrap();

        let a = tempdir.path().join("a");
        std::fs::create_dir(&a).unwrap();
        std::fs::write(a.join("file"), "").unwrap();

        let b = tempdir.path().join("b");
        std::fs::create_dir(&b).unwrap();
        let symlink = b.join("link_to_a");
        std::os::unix::fs::symlink(&a, &symlink).unwrap();

        let request = b.join("**");
        let resolved = resolve_path(&request, follow_links)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 2);
        assert!(resolved.iter().find(|x| x.path == symlink).is_some());
        assert!(resolved
            .iter()
            .find(|x| x.path == symlink.join("file"))
            .is_some());
    }
}