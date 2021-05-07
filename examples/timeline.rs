//! An example that timelines the specified folder.
//!
//! The timeline action is one of the filesystem tools that the GRR agent
//! exposes. Its description is pretty simple: given a folder, it returns a
//! stream of stat entries for all the files in it (including all subfolders).
//!
//! This example is a tiny wrapper around this action that allows to execute it
//! as a standalone binary without all the RRG setup required. A primary reason
//! for its existence is to compare the efficiency Rust implementation against
//! the existing Python one.

use std::fs::File;
use std::path::{Path, PathBuf};

use rrg::action::timeline;
use rrg::session::Sink;
use structopt::StructOpt;

/// A type for the timelining example command-line arguments.
#[derive(StructOpt)]
#[structopt(name = "timeline", about = "A binary for the timeline action.")]
struct Opts {
    /// A path to the root directory to timeline.
    #[structopt(long = "root", name = "FILE", default_value = "/",
                help = "Root directory to timeline")]
    root: PathBuf,

    /// A path to a file to dump the results into.
    #[structopt(long = "output", name = "DIRECTORY",
                help = "File to dump the results into")]
    output: PathBuf,
}

/// A session type of the timeline example.
///
/// Every action handler needs a session object. "Real" sessions handle comms
/// with the GRR server, error reporting and alike. However, this is not needed
/// here and the example session simply dumps everything into the specified file
/// encoding data in the gzchunked format.
struct Session {
    /// A file into which the data chunks of the output are dumped.
    output: File,
}

impl Session {

    /// Creates a session object that will write to the given `output` path.
    fn open<P: AsRef<Path>>(output: P) -> Session {
        let output = File::create(output)
            .expect("failed to create the output file");

        Session {
            output: output,
        }
    }
}

impl rrg::session::Session for Session {

    fn reply<R>(&mut self, response: R) -> rrg::session::Result<()>
    where
        R: rrg::action::Response + 'static,
    {
        // For now we are not interested in doing anything useful with chunk ids
        // since everything is dumped into one file and there is no need to
        // refer to a particular chunk.
        //
        // In the future they might be useful for printing some statistics about
        // the collected files.
        drop(response);

        Ok(())
    }

    fn send<R>(&mut self, sink: Sink, response: R) -> rrg::session::Result<()>
    where
        R: rrg::action::Response + 'static,
    {
        use std::io::Write as _;
        use byteorder::{BigEndian, WriteBytesExt as _};

        // Just a sanity check in case the implementation of the timeline action
        // starts sending data to other sinks.
        assert_eq!(sink, Sink::TRANSFER_STORE);

        let response = (&response as &dyn std::any::Any)
            .downcast_ref::<timeline::Chunk>()
            .expect("unexpected response type");

        self.output.write_u64::<BigEndian>(response.data.len() as u64)
            .expect("failed to write chunk size tag");
        self.output.write_all(&response.data[..])
            .expect("failed to write chunk data");

        Ok(())
    }
}

fn main() {
    let opts = Opts::from_args();

    timeline::handle(&mut Session::open(opts.output), timeline::Request {
        root: opts.root,
    }).expect("failed to execute the action");
}
