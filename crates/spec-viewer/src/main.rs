use std::{net::TcpListener, process::ExitCode, sync::Arc, time::Duration};

mod cli;

use cli::Command;
use spec_viewer_config::{Config, Layer};
use spec_viewer_core::{Library, ParseConfig};
use spec_viewer_daemon::{Daemon, Options, relay, run};
use spec_viewer_protocol::Runtime;
use spec_viewer_render_tui::Theme;
use spec_viewer_render_web::Backend;

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
    let (root, theme) = match cli::parse(&args)? {
        Command::Tui { root, theme } => (root, theme),
        Command::Daemon { idle_exit } => {
            run(Options::from_env(idle_exit.map(Duration::from_secs)), gateway)?;
            return Ok(ExitCode::SUCCESS);
        }
        Command::Relay => {
            relay(&Runtime::from_env(), spawn_daemon()?).map_err(|error| error.to_string())?;
            return Ok(ExitCode::SUCCESS);
        }
    };
    let mut config = Config::load().map_err(|error| error.to_string())?;
    let library = Library::new(&root, ParseConfig::default()).map_err(|error| error.to_string())?;
    let worktree = Layer::read(&library.containing_worktree().path.join(".spec-viewer.toml")).map_err(|error| error.to_string())?;
    config.parse = ParseConfig::with(&[&config.layer, &worktree.unwrap_or_default()]);
    let theme = Theme::from_config(&config, theme).map_err(|error| error.to_string())?;
    let library = library.with_parse_config(config.parse.clone());
    Ok(spec_viewer_render_tui::run(library, theme, config.keys, None, config.tui.diff))
}

/// Serves the web app as an in-process client of `daemon`.
fn gateway(daemon: &Arc<Daemon>, listener: TcpListener) {
    let client = match daemon.connect() {
        Ok(client) => client,
        Err(error) => return eprintln!("spec-viewer: no web app: {error}"),
    };
    let held = Arc::clone(daemon);
    let backend = Backend {
        client,
        hold: Box::new(move || Box::new(held.hold())),
    };
    if let Err(error) = spec_viewer_render_web::start(listener, daemon.config(), backend) {
        eprintln!("spec-viewer: no web app: {error}");
    }
}

/// `<this executable> daemon --idle-exit 300`, as frontends spawn it.
fn spawn_daemon() -> Result<std::process::Command, String> {
    let mut command = std::process::Command::new(std::env::current_exe().map_err(|error| error.to_string())?);
    command.args(["daemon", "--idle-exit", "300"]);
    Ok(command)
}
