use crate::session;
use std::path::PathBuf;
type XDevMode = rrg_proto::file_finder_args::XDev;

/// An error type for failures that can occur during the timeline action.
#[derive(Debug)]
pub enum Error {
    InvalidRecursiveComponentInPath(PathBuf),
    MultipleRecursiveComponentsInPath(PathBuf),
    NonAbsolutePath(PathBuf),
    UnsupportedParameter(String),
    UnsupportedXDevMode(XDevMode),
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
            InvalidRecursiveComponentInPath ( ref path ) => write!(
                fmt,
                "Client Side File Finder path contains an invalid recursive component: {}",
                path.display()
            ),

            MultipleRecursiveComponentsInPath(ref path) => write!(
                fmt,
                "Client Side File Finder path contains more then 1 recursive component: {}",
                path.display()
            ),

            NonAbsolutePath(ref path) => write!(
                fmt,
                "Client Side File Finder path is not absolute: {}",
                path.display()
            ),

            UnsupportedParameter(ref parameter) => write!(
                fmt,
                "Client Side File Finder parameter: {} is not supported",
                parameter
            ),

            UnsupportedXDevMode(ref mode) => write!(
                fmt,
                "Client Side File Finder XDev mode: {:?} is not supported",
                mode
            ),
        }
    }
}

impl From<Error> for session::Error {
    fn from(error: Error) -> session::Error {
        session::Error::action(error)
    }
}
