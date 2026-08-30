//! The `tinyanalyzer` binary.
//!
//! Parse the command line, run one analysis, and hand the report to whichever
//! renderer was asked for. Everything worth testing lives in the library half
//! of this crate; this file is the wiring and the exit code.

use clap::Parser;
use std::process::ExitCode;
use tinyanalyzer::cli::{Cli, Output};
use tinyanalyzer::error::{Error, Result};
use tinyanalyzer::{analyze_with, dashboard, summary};

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tinyanalyzer: {error}");

            // A terminal failure is the one error that can leave the shell
            // unusable, so it gets the one piece of advice worth printing.
            if matches!(error, Error::Terminal { .. }) {
                eprintln!("the terminal may need `reset`");
            }

            ExitCode::FAILURE
        }
    }
}

/// Runs one invocation.
///
/// # Errors
///
/// Returns whatever the analysis, the renderer, or the terminal reported.
fn run(cli: &Cli) -> Result<()> {
    let config = cli.config()?;
    let report = analyze_with(&cli.path, &config)?;

    match cli.output {
        Output::Dashboard => {
            let mut reload_config = config.clone();
            dashboard::run_with_reload(
                report,
                config.ui.start_view,
                config.ui.hide_tests,
                config.scan.respect_gitignore,
                &mut |respect_gitignore| {
                    reload_config.scan.respect_gitignore = respect_gitignore;
                    analyze_with(&cli.path, &reload_config).map_err(Error::from)
                },
            )
        }
        Output::Summary => emit(cli, summary::render(&report, config.ui.hide_tests)),
        Output::Json => emit(cli, report.to_json()?),
    }
}

/// Writes rendered output to the file the operator named, or to standard output.
///
/// # Errors
///
/// Returns [`Error::Write`] if the named file cannot be written.
fn emit(cli: &Cli, rendered: String) -> Result<()> {
    let Some(path) = &cli.write else {
        println!("{rendered}");
        return Ok(());
    };

    std::fs::write(path, rendered).map_err(|source| Error::Write {
        path: path.clone(),
        source,
    })
}
