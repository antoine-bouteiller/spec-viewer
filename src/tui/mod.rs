//! Terminal shell.

use std::path::Path;
use std::process::ExitCode;

use crate::cli::Theme;

pub fn run(_root: &Path, _theme: Option<Theme>) -> ExitCode {
    eprintln!("plan-viewer: the terminal UI is not implemented");
    ExitCode::from(1)
}
