use std::process::ExitCode;

mod cli;

use cli::Command;
use plan_viewer_config::Config;
use plan_viewer_core::{Library, project_of};
use plan_viewer_render_tui::Theme;

fn main() -> ExitCode {
    match start() {
        Ok(code) => code,
        Err(message) => {
            let message = message.split_whitespace().collect::<Vec<_>>().join(" ");
            eprintln!("plan-viewer: {message}");
            ExitCode::from(1)
        }
    }
}

fn start() -> Result<ExitCode, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Command::Tui { root, theme, config } = cli::parse(&args)?;
    let project = project_of(&root).map_err(|error| error.to_string())?;
    let worktree = project
        .worktrees
        .iter()
        .filter(|worktree| root.starts_with(&worktree.path))
        .max_by_key(|worktree| worktree.path.components().count())
        .map_or(root.as_path(), |worktree| worktree.path.as_path());
    let config = Config::load(worktree, config.as_deref()).map_err(|error| error.to_string())?;
    let theme = Theme::from_config(&config, theme).map_err(|error| error.to_string())?;
    let library = Library::new(&root, config.parse).map_err(|error| error.to_string())?;
    Ok(plan_viewer_render_tui::run(library, theme))
}
