// Copyright 2020 Google LLC
//
// Use of this source code is governed by an MIT-style license that can be found
// in the LICENSE file or at https://opensource.org/licenses/MIT.

//! TODO: add a comment

use crate::session::{self, Session};
use rrg_proto::{FileFinderResult, Hash};
use log::info;

type Request = crate::action::client_side_file_finder::request::Request;

#[derive(Debug)]
pub struct Response {
}

pub fn handle<S: Session>(session: &mut S, request: Request) -> session::Result<()> {
    info!("Received client side file finder request request: {:?}", request);

    if request.path_type != PathType:: {
        Err("pathtype parameter is not supported!")
    }
    //
    // if request.conditions.len() > 0{
    //     Err("conditions parameter is not supported!")
    // }
    //
    // if request.process_non_regular_files.is_some() {
    //     Err("process_non_regular_files parameter is not supported!")
    // }
    //
    // if request.follow_links.is_some() {
    //     Err("follow_links parameter is not supported!")
    // }
    //
    // if request.xdev.is_some() {
    //     Err("xdev parameter is not supported!")
    // }

    session.reply(Response {
    })?;
    Ok(())
}

impl super::super::Response for Response {

    const RDF_NAME: Option<&'static str> = Some("FileFinderResult");  // ???

    type Proto = FileFinderResult;

    fn into_proto(self) -> FileFinderResult {
        FileFinderResult{
            hash_entry: Some(Hash{
                sha256: Some(vec!(1,2,3,4,5,6,7,8,9,10)),  // TODO: just a test
                ..Default::default()
            }),
            matches: vec!(),
            stat_entry: None,
            transferred_file: None
        }
    }
}

#[cfg(test)]
mod tests {
    // use super::*;

    #[test]
    fn test() {
        // let mut session = session::test::Fake::new();
        // let request = Request2{paths: vec!("SOME_PATH".to_owned()), action: Some(Action::Stat{})};
        // assert!(handle(&mut session, request).is_ok());
        //
        // let args : FileFinderArgs = FileFinderArgs{
        //     action: Some(rrg_proto::FileFinderAction{
        //         // action_type: Some(2),
        //         action_type: None,
        //         ..Default::default()}
        //     ),
        //         ..Default::default()};
        //
        // let req = Request2::from_proto(args);
        // println!("req: {:?}", req);

        // assert_eq!(session.reply_count(), 1);
    }
}

// TODO: create a dir for this action and do request/response in separate files from the main logic
// TODO: tests on a real FS

// TODO: support %%code_page%% and others like that
