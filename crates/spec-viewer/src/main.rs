use std::{net::TcpListener, process::ExitCode, sync::Arc, time::Duration};

mod cli;

use cli::Command;
use spec_viewer_config::Config;
use spec_viewer_daemon::{Daemon, Options, relay, run};
use spec_viewer_protocol::{Client, Error, Runtime};
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
    let (root, host, theme) = match cli::parse(&args)? {
        Command::Tui { root, host, theme } => (root, host, theme),
        Command::Daemon { idle_exit } => {
            run(Options::from_env(idle_exit.map(Duration::from_secs)), gateway)?;
            return Ok(ExitCode::SUCCESS);
        }
        Command::Relay => {
            relay(&Runtime::from_env(), spawn_daemon()?).map_err(|error| error.to_string())?;
            return Ok(ExitCode::SUCCESS);
        }
    };
    let config = Config::load().map_err(|error| error.to_string())?;
    let theme = Theme::from_config(&config, theme).map_err(|error| error.to_string())?;
    let client = Client::connect().map_err(|error| match error {
        Error::Protocol { daemon, client } => format!("the daemon on this host speaks protocol {daemon}, this client {client}"),
        error => error.to_string(),
    })?;
    Ok(spec_viewer_render_tui::run(
        client,
        host.as_deref(),
        &root,
        theme,
        config.keys,
        config.tui.diff,
    ))
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
