// Copyright 2020 Google LLC
//
// Use of this source code is governed by an MIT-style license that can be found
// in the LICENSE file or at https://opensource.org/licenses/MIT.

//! Handler for `client side file finder` action.
//!
//! The basic functionality of this handler is to return information
//! about the filesystem object with given path.
//! Features supported by this handler:
//! - Resolve glob expressions inb paths e.g. `/a?[!d]*` match `/abcd`.
//! - Resolve recursive elements in glob expressions e.g. `/**` match `/a/b`.
//! - Resolve alternatives in paths e.g. `/a{b,c}d` match `/abd` and `/acd`.
//!
//! Expression expansion (like `%%hostname%%` visible in the GRR Admin UI)
//! is performed on the server side, so it's out of scope of this handler.

pub mod action;
pub mod chunks;
pub mod condition;
pub mod download;
pub mod file;
pub mod glob;
pub mod groups;
pub mod hash;
pub mod path;
pub mod request;
pub mod task;

// Life of a path:
// - in the input proto FileFinderArgs::paths are `String`.
// - path groups are expanded - the type is still a `String`
// - path is converted to `Task`, in which the constant part of the path is
//   stored as a `PathBuf`, glob parts are stored as a `Regex`.
