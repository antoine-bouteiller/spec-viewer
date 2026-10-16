//! HTTP shell: drop-in for `bun src/server.ts`.

use std::path::Path;
use std::process::ExitCode;

pub fn run(_root: &Path, _port: u16) -> ExitCode {
    eprintln!("plan-viewer: serve is not implemented");
    ExitCode::from(1)
}
