use std::process::ExitCode;

mod cli;

use cli::Command;
use spec_viewer_config::{Config, Layer};
use spec_viewer_core::{Library, ParseConfig};
use spec_viewer_render_tui::Theme;

fn main() -> ExitCode {
    match start() {
        Ok(code) => code,
        Err(message) => {
            let message = message.split_whitespace().collect::<Vec<_>>().join(" ");
            eprintln!("spec-viewer: {message}");
            ExitCode::from(1)
        }
    }
}

fn start() -> Result<ExitCode, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Command::Tui { root, theme } = cli::parse(&args)?;
    let mut config = Config::load().map_err(|error| error.to_string())?;
    let library = Library::new(&root, ParseConfig::default()).map_err(|error| error.to_string())?;
    let worktree = Layer::read(&library.containing_worktree().path.join(".spec-viewer.toml")).map_err(|error| error.to_string())?;
    config.parse = ParseConfig::with(&[&config.layer, &worktree.unwrap_or_default()]);
    let theme = Theme::from_config(&config, theme).map_err(|error| error.to_string())?;
    let web = Library::new(&root, config.parse.clone()).map_err(|error| error.to_string())?;
    let library = library.with_parse_config(config.parse.clone());
    let address = spec_viewer_render_web::start(web, &config, None).map_err(|error| format!("cannot start web server: {error}"))?;
    Ok(spec_viewer_render_tui::run(
        library,
        theme,
        config.keys,
        Some(format!("http://{address}")),
        config.tui.diff,
    ))
}
