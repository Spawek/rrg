use lazy_static::lazy_static;
use regex::Regex;
use crate::action::finder::glob::glob_to_regex;
use std::path::{PathBuf, Path};

/// Part of the path. Paths are split to list of `PathComponent` to make
/// the processing simpler.
#[derive(Debug, Clone)]
pub enum PathComponent {
    Constant(PathBuf),  // e.g. `/home/user/`
    Glob(Regex),  // e.g. `sp*[wek]??`
    RecursiveScan {max_depth: i32},  // glob recursive component - `**` in path
}

/// File finder task to be executed.
#[derive(Debug)]
pub struct Task {
    /// Path prefix in which scope the task must be executed.
    /// Given example task: `/a/b/**4/c/d*` this part would be `/a/b`.
    /// Given example task: `/a/b/c` this part would be empty.
    pub path_prefix: PathBuf,

    /// Current `PathComponent` to be executed.
    /// Given example task: `/a/b/**4/c/d*` this part would be `**4`.
    /// Given example task: `/a/b/c` this part would be `/a/b/c`.
    pub current_component : PathComponent,

    /// Remaining path components to be executed in following tasks.
    /// Given example task: `/a/b/**4/c/d*` this part would be `c/d*`.
    /// Given example task: `/a/b/c` this part would be empty.
    pub remaining_components : Vec<PathComponent>,
}

pub struct TaskBuilder {
    components: Vec<PathComponent>,
}

impl TaskBuilder {
    pub fn new() -> TaskBuilder {
        TaskBuilder { components: vec![] }
    }

    pub fn add_constant(mut self, path: &Path) -> TaskBuilder {
        self.components.push(PathComponent::Constant(path.to_path_buf()));
        self
    }

    pub fn add_recursive_scan(mut self, max_depth: i32) -> TaskBuilder {
        self.components.push(PathComponent::RecursiveScan{max_depth});
        self
    }


    pub fn add_components(mut self, components: Vec<PathComponent>) -> TaskBuilder {
        self.components.extend(components);
        self
    }

    pub fn build(self) -> Task{
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
    for i in 0..folded_components.len(){
        let component = folded_components.get(i).unwrap();
        match component{
            PathComponent::Constant(c) => {
                path_prefix = c.to_owned();
            },
            v @ PathComponent::Glob(_) => {
                let remaining_components = folded_components[i+1..]
                    .into_iter().map(|x| x.to_owned()).collect();
                return Task {path_prefix, current_component: v.clone(), remaining_components}
            },
            v @ PathComponent::RecursiveScan {..} => {
                let remaining_components = folded_components[i+1..]
                    .into_iter().map(|x| x.to_owned()).collect();
                return Task {path_prefix, current_component: v.clone(), remaining_components}
            },
        }
    }

    Task {
        path_prefix: PathBuf::default(),
        current_component: PathComponent::Constant(path_prefix.to_owned()),
        remaining_components: vec![],
    }
}

// TODO: take PathBuf here and use Path::components
pub fn build_task(path: &str) -> Task {
    if !path.starts_with(&"/") {
        panic!("path must be absolute");  // TODO: throw a meaningful error
    }

    // adds the root dir // TODO: rethink this comment
    let mut components : Vec<PathComponent> = vec![PathComponent::Constant(PathBuf::from("/"))];  // TODO: is it needed actually
    components.extend(path.split("/").into_iter()  // TODO: support different OS separators
        .filter(|x| !x.is_empty())
        .map(get_path_component));

    build_task_from_components(fold_constant_components(components))
}

fn get_path_component(s : &str) -> PathComponent {
    let recursive_scan = get_recursive_scan_component(s);
    if recursive_scan.is_some(){
        return recursive_scan.unwrap();
    }

    let glob = get_glob_component(s);
    if glob.is_some(){
        return glob.unwrap();
    }

    PathComponent::Constant(PathBuf::from(s))
}

fn get_recursive_scan_component(s : &str) -> Option<PathComponent>{
    const DEFAULT_DEPTH : i32 = 3;

    lazy_static!{
        static ref RECURSIVE_SCAN_MATCHER : Regex = Regex::new(r"\*\*(?P<max_depth>\d*)").unwrap();
    }

    match RECURSIVE_SCAN_MATCHER.captures(s){
        Some(m) => {
            let max_depth = if m["max_depth"].is_empty()
            {
                DEFAULT_DEPTH
            }
            else {
                let v = m["max_depth"].parse::<i32>();
                if v.is_err(){
                    return None;  // TODO: throw some error
                }
                v.unwrap()
            };
            Some(PathComponent::RecursiveScan {max_depth})
        }
        None => {return None;}
    }

    // TODO: throw ValueError("malformed recursive component") when there is something more in the match
}

fn get_glob_component(s : &str) -> Option<PathComponent>{
    lazy_static!{
        static ref GLOB_MATCHER : Regex = Regex::new(r"\*|\?|\[.+\]").unwrap();
    }

    if !GLOB_MATCHER.is_match(s){
        return None;
    }

    match glob_to_regex(s){
        Ok(regex) => Some(PathComponent::Glob(regex)),
        Err(_) => None,  // TODO: handle error case somehow
    }
}

pub fn get_constant_component_value(constant_component: &PathComponent) -> PathBuf {
    match constant_component{
        PathComponent::Constant(s) => s.to_owned(),
        _ => panic!()
    }
}

pub fn fold_constant_components(components: Vec<PathComponent>) -> Vec<PathComponent>{
    let mut ret = vec![];
    for c in components {
        if !ret.is_empty() && matches!(ret.last().unwrap(), PathComponent::Constant(_)) && matches!(&c, PathComponent::Constant(_)) {
            let prev_last = ret.swap_remove(ret.len() - 1);
            ret.push(PathComponent::Constant(
                get_constant_component_value(&prev_last)
                    .join(&get_constant_component_value(&c))));
        }
        else {
            ret.push(c.clone());
        }
    }

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_glob_component(component: &PathComponent, expected_regex: &str){
        match component {
            PathComponent::Glob(regex) => {assert_eq!(regex.as_str(), expected_regex);},
            _ => {panic!("expected glob component: {}, got: {:?}", expected_regex, component)}
        }
    }

    fn assert_recursive_scan_component(component: &PathComponent, expected_depth: i32){
        match component {
            PathComponent::RecursiveScan{max_depth} => {assert_eq!(max_depth, &expected_depth);},
            _ => {panic!("expected recursive scan component: {}, got: {:?}", expected_depth, component)}
        }
    }

    #[test]
    fn basic_parse_path_test() {
        let task = build_task("/home/user/**5/??[!qwe]");
        assert_eq!(task.path_prefix, PathBuf::from("/home/user"));
        assert_recursive_scan_component(&task.current_component, 5);
        assert_eq!(task.remaining_components.len(), 1);
        assert_glob_component(&task.remaining_components[0], "^..[^qwe]$");
    }

    #[test]
    fn default_glob_depth_test() {
        let task = build_task("/**");
        assert_eq!(task.path_prefix, PathBuf::from("/"));
        assert_recursive_scan_component(&task.current_component, 3);
        assert_eq!(task.remaining_components.len(), 0);
    }
}
