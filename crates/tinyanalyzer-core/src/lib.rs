//! placeholder
#![allow(missing_docs)]
pub mod config;
pub mod dead_code;
pub mod deps;
pub mod error;
pub mod findings;
pub mod loc;
pub mod report;
pub mod rust_source;
pub mod walk;
pub use config::Config;
pub use error::{Error, Result};
pub use loc::{Language, LineCounts, count_lines};
pub use report::Report;
