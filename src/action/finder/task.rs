use crate::action::finder::error::Error;
use crate::action::finder::glob::glob_to_regex;
use lazy_static::lazy_static;
use regex::Regex;
use std::path::{Component, Path, PathBuf};

/// Part of the path. Paths are split to list of `PathComponent` to make
/// the processing simpler.
#[derive(Debug, Clone)]
pub enum PathComponent {
    Constant(PathBuf),                // e.g. `/home/user/`
    Glob(Regex),                      // e.g. `sp*[wek]??`
    RecursiveScan { max_depth: i32 }, // glob recursive component - `**` in path
}

/// Internal path representation used for resolving paths.
/// E.g. `/home/**1/*/test` path would be stored as:
/// path_prefix: `/home`,
/// current_component: `**1`,
/// remaining_components: [`*`, `test`].
#[derive(Debug)]
pub struct Task {
    /// Path prefix in which scope the task must be executed.
    /// Given example task: `/a/b/**4/c/d*` this part would be `/a/b`.
    /// Given example task: `/a/b/c` this part would be empty.
    pub path_prefix: PathBuf,

    /// Current `PathComponent` to be executed.
    /// Given example task: `/a/b/**4/c/d*` this part would be `**4`.
    /// Given example task: `/a/b/c` this part would be `/a/b/c`.
    pub current_component: PathComponent,

    /// Remaining path components to be executed in following tasks.
    /// Given example task: `/a/b/**4/c/d*` this part would be `c/d*`.
    /// Given example task: `/a/b/c` this part would be empty.
    pub remaining_components: Vec<PathComponent>,
}

pub struct TaskBuilder {
    components: Vec<PathComponent>,
}

impl TaskBuilder {
    pub fn new() -> TaskBuilder {
        TaskBuilder { components: vec![] }
    }

    pub fn add_constant(mut self, path: &Path) -> TaskBuilder {
        self.components
            .push(PathComponent::Constant(path.to_path_buf()));
        self
    }

    pub fn add_recursive_scan(mut self, max_depth: i32) -> TaskBuilder {
        self.components
            .push(PathComponent::RecursiveScan { max_depth });
        self
    }

    pub fn add_components(
        mut self,
        components: Vec<PathComponent>,
    ) -> TaskBuilder {
        self.components.extend(components);
        self
    }

    pub fn build(self) -> Task {
        build_task_from_components(self.components)
    }
}

impl From<TaskBuilder> for Task {
    fn from(builder: TaskBuilder) -> Task {
        build_task_from_components(builder.components)
    }
}

fn build_task_from_components(components: Vec<PathComponent>) -> Task {
    let folded_components = fold_constant_components(components);

    // Scan components until an non-const component or the end of path.
    let mut path_prefix = PathBuf::default();
    for i in 0..folded_components.len() {
        let component = folded_components.get(i).unwrap();
        match component {
            PathComponent::Constant(c) => {
                path_prefix = c.to_owned();
            }
            v @ PathComponent::Glob(_) => {
                let remaining_components = folded_components[i + 1..]
                    .into_iter()
                    .map(|x| x.to_owned())
                    .collect();
                return Task {
                    path_prefix,
                    current_component: v.clone(),
                    remaining_components,
                };
            }
            v @ PathComponent::RecursiveScan { .. } => {
                let remaining_components = folded_components[i + 1..]
                    .into_iter()
                    .map(|x| x.to_owned())
                    .collect();
                return Task {
                    path_prefix,
                    current_component: v.clone(),
                    remaining_components,
                };
            }
        }
    }

    Task {
        path_prefix: PathBuf::default(),
        current_component: PathComponent::Constant(path_prefix.to_owned()),
        remaining_components: vec![],
    }
}

pub fn build_task(path: &Path) -> Result<Task, Error> {
    let components = path
        .components()
        .map(|x| get_path_component(&x))
        .collect::<Result<Vec<PathComponent>, Error>>()?;

    if components
        .iter()
        .filter(|x| matches!(x, PathComponent::RecursiveScan {..}))
        .count()
        > 1
    {
        return Err(Error::MultipleRecursiveComponentsInPath(
            path.to_path_buf(),
        ));
    }

    Ok(build_task_from_components(fold_constant_components(
        components,
    )))
}

fn get_path_component(component: &Component) -> Result<PathComponent, Error> {
    let s = match component {
        Component::Normal(path) => match path.to_str() {
            Some(s) => s,
            None => {
                return Ok(PathComponent::Constant(PathBuf::from(component)))
            }
        },
        _ => return Ok(PathComponent::Constant(PathBuf::from(component))),
    };

    if let Some(scan) = get_recursive_scan_component(s)? {
        return Ok(scan);
    }

    if let Some(glob) = get_glob_component(s) {
        return Ok(glob);
    }

    Ok(PathComponent::Constant(PathBuf::from(s)))
}

fn get_recursive_scan_component(
    s: &str,
) -> Result<Option<PathComponent>, Error> {
    const DEFAULT_DEPTH: i32 = 3;

    lazy_static! {
        static ref RECURSIVE_SCAN_MATCHER: Regex =
            Regex::new(r"\*\*(?P<max_depth>\d*)(?P<remaining>.*)").unwrap();
    }

    let captures = match RECURSIVE_SCAN_MATCHER.captures(s) {
        Some(captures) => captures,
        None => return Ok(None),
    };

    if !captures["remaining"].is_empty() {
        return Err(Error::InvalidRecursiveComponentInPath(PathBuf::from(s)));
    }

    let max_depth = match &captures["max_depth"] {
        "" => DEFAULT_DEPTH,
        val @ _ => match val.parse::<i32>() {
            Ok(v) => v,
            Err(_) => {
                return Err(Error::InvalidRecursiveComponentInPath(
                    PathBuf::from(s),
                ));
            }
        },
    };

    Ok(Some(PathComponent::RecursiveScan { max_depth }))
}

fn get_glob_component(s: &str) -> Option<PathComponent> {
    lazy_static! {
        static ref GLOB_MATCHER: Regex = Regex::new(r"\*|\?|\[.+\]").unwrap();
    }

    if !GLOB_MATCHER.is_match(s) {
        return None;
    }

    match glob_to_regex(s) {
        Ok(regex) => Some(PathComponent::Glob(regex)),
        Err(_) => None, // TODO: handle error case somehow
    }
}

pub fn get_constant_component_value(
    constant_component: &PathComponent,
) -> PathBuf {
    match constant_component {
        PathComponent::Constant(s) => s.to_owned(),
        _ => panic!(),
    }
}

pub fn fold_constant_components(
    components: Vec<PathComponent>,
) -> Vec<PathComponent> {
    let mut ret = vec![];
    for c in components {
        if !ret.is_empty()
            && matches!(ret.last().unwrap(), PathComponent::Constant(_))
            && matches!(&c, PathComponent::Constant(_))
        {
            let prev_last = ret.swap_remove(ret.len() - 1);
            ret.push(PathComponent::Constant(
                get_constant_component_value(&prev_last)
                    .join(&get_constant_component_value(&c)),
            ));
        } else {
            ret.push(c.clone());
        }
    }

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_parse_path() {
        let task = build_task(
            &PathBuf::new()
                .join(Component::RootDir)
                .join("home")
                .join("user")
                .join("**5")
                .join("??[!qwe]"),
        )
        .unwrap();

        assert_eq!(task.path_prefix, PathBuf::from("/home/user"));
        assert!(matches!(
            &task.current_component,
            PathComponent::RecursiveScan { max_depth: 5 }
        ));
        assert_eq!(task.remaining_components.len(), 1);
        assert!(
            matches!(&task.remaining_components[0], PathComponent::Glob(regex) if regex.as_str() == "^..[^qwe]$")
        );
    }

    #[test]
    fn test_default_recursive_scan_default_depth() {
        let task =
            build_task(&PathBuf::new().join(Component::RootDir).join("**"))
                .unwrap();
        assert_eq!(task.path_prefix, PathBuf::from("/"));
        assert!(matches!(
            &task.current_component,
            PathComponent::RecursiveScan { max_depth: 3 }
        ));
        assert_eq!(task.remaining_components.len(), 0);
    }

    #[test]
    fn test_recursive_scan_with_additional_letters() {
        let task =
            build_task(&PathBuf::new().join(Component::RootDir).join("**5asd"));
        assert!(
            matches!(task.unwrap_err(),
            Error::InvalidRecursiveComponentInPath(path)
            if path == PathBuf::from("**5asd"))
        );
    }

    #[test]
    fn test_path_with_multiple_recursive_scans() {
        let path = PathBuf::new().join(Component::RootDir).join("**/asd/**");
        let task = build_task(&path);
        assert!(
            matches!(task.unwrap_err(),
             error::MultipleRecursiveComponentsInPath(err_path)
             if err_path == path)
        );
    }
}
