use lazy_static::lazy_static;
use regex::Regex;
use crate::action::client_side_file_finder::glob_to_regex::glob_to_regex;

/// Task is split into parts to make the execution simpler.
#[derive(Debug)]
pub struct TaskDetails {
    /// Path prefix in which scope the task must be executed.
    /// Given example task: `/a/b/**4/c/d*` this part would be `/a/b`.
    /// Given example task: `/a/b/c` this part would be empty.
    pub path_prefix: String,

    /// Current `PathComponent` to be executed.
    /// Given example task: `/a/b/**4/c/d*` this part would be `/**4`.
    /// Given example task: `/a/b/c` this part would be `/a/b/c`.
    pub current_component : PathComponent,

    /// Remaining path components to be executed in following tasks.
    /// Given example task: `/a/b/**4/c/d*` this part would be `c/d*`.
    /// Given example task: `/a/b/c` this part would be empty.
    pub remaining_components : Vec<PathComponent>,
}

pub fn get_task_details(task: &Path) -> TaskDetails {
    let folded_components = fold_constant_components(&task.components);
    println!("folded components: {:?}", folded_components);

    // Scan components until an non-const component or the end of path.
    let mut path_prefix = "".to_owned();
    for i in 0..folded_components.len(){
        let component = folded_components.get(i).unwrap();
        match component{
            PathComponent::Constant(c) => {
                path_prefix = c.to_owned();
            },
            v @ PathComponent::Glob(_) => {
                let remaining_components = folded_components[i+1..]
                    .into_iter().map(|x| x.to_owned()).collect();
                return TaskDetails{path_prefix, current_component: v.clone(), remaining_components}
            },
            v @ PathComponent::RecursiveScan {..} => {
                let remaining_components = folded_components[i+1..]
                    .into_iter().map(|x| x.to_owned()).collect();
                return TaskDetails{path_prefix, current_component: v.clone(), remaining_components}

            },
        }
    }

    TaskDetails {
        path_prefix: "".to_owned(),
        current_component: PathComponent::Constant(path_prefix.to_owned()),
        remaining_components: vec![]
    }
}

//TODO: remove it and use TaskDetails instead right away
//TODO: rename to Task?
#[derive(Debug, Clone)]
pub struct Path {
    pub components : Vec<PathComponent>
}

#[derive(Debug, Clone)]
pub enum PathComponent {
    Constant(String),  // e.g. `/home/user/`
    Glob(Regex),  // glob expression e.g. `sp*[wek]??`
    RecursiveScan {max_depth: i32},  // glob recursive component - `**` in path
}

pub fn parse_path(path: &str) -> Path {
    if !path.starts_with(&"/") {
        panic!("path must be absolute");  // TODO: throw a meaningful error
    }
    let split : Vec<&str> = path.split("/").collect();  // TODO: support different OS separators
    let mut components : Vec<PathComponent> = split.into_iter()
        .filter(|x| !x.is_empty())
        .map(get_path_component)
        .collect();

    components.insert(0, PathComponent::Constant("".to_owned())); // will add "/" at the beginning
    let components = fold_constant_components(&components);

    Path{components}
}

fn get_path_component(s : &str) -> PathComponent {
    let recursive_scan = get_recursive_scan_component(s);
    if recursive_scan.is_some(){
        return recursive_scan.unwrap();
    }

    let scan = get_scan_component(s);
    if scan.is_some(){
        return scan.unwrap();
    }

    PathComponent::Constant(s.to_owned())
}

fn get_recursive_scan_component(s : &str) -> Option<PathComponent>{
    const DEFAULT_DEPTH : i32 = 3;

    lazy_static!{
        static ref RE : Regex = Regex::new(r"\*\*(?P<max_depth>\d*)").unwrap();
    }

    match RE.captures(s){
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

fn get_scan_component(s : &str) -> Option<PathComponent>{
    lazy_static!{
        static ref RE : Regex = Regex::new(r"\*|\?|\[.+\]").unwrap();
    }

    if !RE.is_match(s){
        return None;
    }

    match glob_to_regex(s){
        Ok(regex) => Some(PathComponent::Glob(regex)),
        Err(_) => None,  // TODO: handle error case somehow
    }
}

pub fn is_constant_component(component: &PathComponent) -> bool {
    match component{
        PathComponent::Constant(_) => true,
        _ => false
    }
}

pub fn get_constant_component_value(constant_component: &PathComponent) -> String {
    match constant_component{
        PathComponent::Constant(s) => s.to_owned(),
        _ => panic!()
    }
}

pub fn fold_constant_components(components: &Vec<PathComponent>) -> Vec<PathComponent>{
    let mut ret = vec![];
    for c in components {
        if !ret.is_empty() && is_constant_component(ret.last().unwrap()) && is_constant_component(&c) {
            let prev_last = ret.swap_remove(ret.len() - 1);
            ret.push(PathComponent::Constant(
                get_constant_component_value(&prev_last)
                + "/"
                + &get_constant_component_value(&c)));  // TODO: set "/" to proper value
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

    fn assert_constant_component(component: &PathComponent, expected_value: &str){
        match component {
            PathComponent::Constant(c) => {assert_eq!(c, expected_value);},
            _ => {panic!("expected constant component: {}, got: {:?}", expected_value, component)}
        }
    }

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
        let components = parse_path("/home/spawek/**5/??[!qwe]").components;
        assert_eq!(components.len(), 3);
        assert_constant_component(&components[0], "/home/spawek");
        assert_recursive_scan_component(&components[1], 5);
        assert_glob_component(&components[2], "..[^qwe]");
    }

    #[test]
    fn default_glob_depth_test() {
        let components = parse_path("/**").components;
        assert_eq!(components.len(), 1);
        assert_recursive_scan_component(&components[0], 3);
    }
}
