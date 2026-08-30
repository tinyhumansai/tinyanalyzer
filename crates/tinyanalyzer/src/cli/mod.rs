//! The command line.
//!
//! One command with one required idea — a path to analyze — and a small set of
//! flags that either change what is measured or change where the answer goes.
//! There are no subcommands on purpose: everything this program does is one
//! analysis, and the only real question is what you want done with it.
//!
//! Flags override the configuration file rather than replacing it. A run with
//! `--no-deps` against a repository whose `tinyanalyzer.toml` sets thresholds
//! keeps those thresholds; the flag turns off one pass, not the file.

use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use tinyanalyzer_core::{Config, Result, StartView};

/// Analyze a Rust codebase and explore it in a terminal dashboard.
#[derive(Debug, Clone, Parser)]
#[command(name = "tinyanalyzer", version, about, long_about = None)]
pub struct Cli {
    /// Repository to analyze.
    #[arg(default_value = ".", value_name = "PATH")]
    pub path: PathBuf,

    /// Read configuration from this file instead of looking for one in PATH.
    #[arg(long, short, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// What to do with the report.
    #[arg(long, short, value_enum, default_value_t = Output::Dashboard)]
    pub output: Output,

    /// Write the report to a file instead of standard output.
    ///
    /// Only meaningful with `--output json` or `--output summary`.
    #[arg(long, value_name = "FILE")]
    pub write: Option<PathBuf>,

    /// Open the dashboard on this view.
    #[arg(long, value_enum, value_name = "VIEW")]
    pub view: Option<View>,

    /// Exclude test code from every total on startup.
    #[arg(long)]
    pub hide_tests: bool,

    /// Skip the dependency graph.
    ///
    /// Makes the run pure filesystem work, which is what you want against a
    /// tree that does not resolve, or in a hook where launching cargo is too
    /// slow to be worth it.
    #[arg(long)]
    pub no_deps: bool,

    /// Skip dead-code detection.
    #[arg(long)]
    pub no_dead_code: bool,

    /// Include hidden files and directories.
    #[arg(long)]
    pub hidden: bool,

    /// Analyze files that ignore rules would otherwise exclude.
    #[arg(long)]
    pub no_ignore: bool,
}

/// What to do with the report once it exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
#[value(rename_all = "kebab-case")]
pub enum Output {
    /// Open the interactive terminal dashboard.
    #[default]
    Dashboard,
    /// Print a ranked text summary and exit.
    Summary,
    /// Print the whole report as JSON and exit.
    Json,
}

/// The dashboard view to open on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum View {
    /// Totals, language mix, and the headline findings.
    Overview,
    /// Files ranked by weight.
    Files,
    /// The dependency graph and the heaviest crates in it.
    Dependencies,
    /// Unreferenced items.
    DeadCode,
    /// Every finding, ranked by severity.
    Findings,
}

impl From<View> for StartView {
    fn from(view: View) -> Self {
        match view {
            View::Overview => Self::Overview,
            View::Files => Self::Files,
            View::Dependencies => Self::Dependencies,
            View::DeadCode => Self::DeadCode,
            View::Findings => Self::Findings,
        }
    }
}

impl Cli {
    /// Builds the configuration this invocation should run with.
    ///
    /// Loads the file — the one named by `--config`, or whichever the analysis
    /// root holds — and then applies the flags on top of it.
    ///
    /// # Errors
    ///
    /// Returns [`tinyanalyzer_core::Error::Io`] if a configuration file cannot
    /// be read and [`tinyanalyzer_core::Error::Config`] if it does not parse.
    pub fn config(&self) -> Result<Config> {
        let mut config = match &self.config {
            Some(path) => Config::from_file(path)?,
            None => Config::load(&self.path)?,
        };

        if self.no_deps {
            config.dependencies.enabled = false;
        }
        if self.no_dead_code {
            config.dead_code.enabled = false;
        }
        if self.hidden {
            config.scan.include_hidden = true;
        }
        if self.no_ignore {
            config.scan.respect_gitignore = false;
        }
        if self.hide_tests {
            config.ui.hide_tests = true;
        }
        if let Some(view) = self.view {
            config.ui.start_view = view.into();
        }

        Ok(config)
    }
}

#[cfg(test)]
mod test;
