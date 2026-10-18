use std::process::ExitCode;

mod cli;

use cli::Command;
use plan_viewer_config::Config;
use plan_viewer_core::{Library, ParseConfig};
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
    let library = Library::new(&root, ParseConfig::default()).map_err(|error| error.to_string())?;
    let config = Config::load(&library.containing_worktree().path, config.as_deref()).map_err(|error| error.to_string())?;
    let theme = Theme::from_config(&config, theme).map_err(|error| error.to_string())?;
    let library = library.with_parse_config(config.parse);
    Ok(plan_viewer_render_tui::run(library, theme))
}
