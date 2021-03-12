use crate::session;
use std::path::PathBuf;

/// An error type for failures that can occur during the timeline action.
#[derive(Debug)]
pub enum Error {
    InvalidRecursiveComponentInPath { path: String },
    MultipleRecursiveComponentsInPath(PathBuf),
}

// TODO: needed?
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        use Error::*;

        match *self {
            InvalidRecursiveComponentInPath { ref path } => write!(
                fmt,
                "Path contains an invalid recursive component: {}",
                path
            ),

            MultipleRecursiveComponentsInPath (ref path) => write!(
                fmt,
                "Path contains more then 1 recursive component: {}",
                path.display()
            ),
        }
    }
}

impl From<Error> for session::Error {
    fn from(error: Error) -> session::Error {
        session::Error::action(error)
    }
}
