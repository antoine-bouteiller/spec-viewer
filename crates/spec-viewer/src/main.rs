use std::process::ExitCode;

mod cli;

use cli::Command;
use spec_viewer_config::{Config, Ports};
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
    let Command::Tui { root, theme, config } = cli::parse(&args)?;
    let library = Library::new(&root, ParseConfig::default()).map_err(|error| error.to_string())?;
    let config = Config::load(&library.containing_worktree().path, config.as_deref()).map_err(|error| error.to_string())?;
    let theme = Theme::from_config(&config, theme).map_err(|error| error.to_string())?;
    let web = Library::new(&root, config.parse.clone()).map_err(|error| error.to_string())?;
    let library = library.with_parse_config(config.parse.clone());
    let url = serve(web, &config, Ports::user())?;
    Ok(spec_viewer_render_tui::run(library, theme, Some(url)))
}

/// Serves the web app on the project's stored port, storing the first port the OS assigns.
fn serve(library: Library, config: &Config, ports: Option<Ports>) -> Result<String, String> {
    let key = library.project().key;
    let stored = ports.as_ref().and_then(|ports| ports.get(&key));
    let address = spec_viewer_render_web::start(library, config, stored).map_err(|error| format!("cannot start web server: {error}"))?;
    if stored.is_none()
        && let Some(ports) = ports
    {
        // An unwritable config directory only costs a stable port.
        let _ = ports.set(&key, address.port());
    }
    Ok(format!("http://{address}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_assigned_port_is_stored_and_kept_when_taken() {
        let root = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        let ports = Ports::at(user.path().join("spec-viewer/ports.toml"));
        let library = || Library::new(root.path(), ParseConfig::default()).unwrap();
        let key = library().project().key;
        let first = serve(library(), &Config::default(), Some(ports.clone())).unwrap();
        let stored = ports.get(&key).unwrap();
        assert_eq!(first, format!("http://127.0.0.1:{stored}"));
        // The first server still holds the stored port, so this one falls back without overwriting it.
        let second = serve(library(), &Config::default(), Some(ports.clone())).unwrap();
        assert_ne!(second, first);
        assert_eq!(ports.get(&key), Some(stored));
    }
}
