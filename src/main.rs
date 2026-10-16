use std::process::ExitCode;

use plan_viewer::cli::{self, Command};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args) {
        Err(message) => {
            eprintln!("plan-viewer: {message}");
            ExitCode::from(1)
        }
        Ok(Command::Tui { root, theme }) => plan_viewer::tui::run(&root, theme),
        Ok(Command::Serve { root, port }) => plan_viewer::serve::run(&root, port),
    }
}
