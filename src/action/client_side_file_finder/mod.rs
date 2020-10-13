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
//!
//! Compontents:
//! - `action.rs` defines a handler and covers the main functionality.
//! - `request.rs` parses protobuf request to the internal representation.
//! - `glob_to_regex.rs` converts glob expressions to `regex::Regex`.
//! - `resolve_path_alternatives.rs` performs groups expansions.
//! - `path.rs` parses paths to internal representation.

pub mod action;

mod request;
mod glob_to_regex;
mod resolve_path_alternatives;
mod path;
