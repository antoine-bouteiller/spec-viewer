use std::process::ExitCode;

mod cli;

use cli::Command;
use plan_viewer_render_tui::terminal::Theme;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args) {
        Err(message) => {
            eprintln!("plan-viewer: {message}");
            ExitCode::from(1)
        }
        Ok(Command::Tui { root, theme }) => plan_viewer_render_tui::run(
            &root,
            theme.map(|theme| match theme {
                cli::Theme::Light => Theme::Light,
                cli::Theme::Dark => Theme::Dark,
            }),
        ),
    }
}
