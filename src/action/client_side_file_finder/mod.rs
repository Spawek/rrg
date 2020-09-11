// Copyright 2020 Google LLC
//
// Use of this source code is governed by an MIT-style license that can be found
// in the LICENSE file or at https://opensource.org/licenses/MIT.

//! Handler for `client side file finder` action.

pub mod action;

mod request;
mod glob_to_regex;
mod expand_groups;
mod path;
