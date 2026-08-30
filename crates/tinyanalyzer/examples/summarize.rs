//! Analyze a directory and print the ranked summary.
//!
//! The smallest useful program that embeds the analyzer: one call to
//! [`analyze`], one call to [`summary::render`]. Everything the binary adds on
//! top of this is a command line and a terminal.
//!
//! ```sh
//! cargo run -p tinyanalyzer --example summarize
//! cargo run -p tinyanalyzer --example summarize -- ../some/other/repo
//! ```

use std::process::ExitCode;
use tinyanalyzer::{analyze, summary};

fn main() -> ExitCode {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".to_owned());

    match analyze(&root) {
        Ok(report) => {
            print!("{}", summary::render(&report, true));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("cannot analyze {root}: {error}");
            ExitCode::FAILURE
        }
    }
}
