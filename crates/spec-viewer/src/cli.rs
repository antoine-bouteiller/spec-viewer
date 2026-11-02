use std::path::PathBuf;

use spec_viewer_config::Mode as Theme;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// `root` is absolute on `host`, the configured remote host, else resolved here.
    Tui {
        root: PathBuf,
        host: Option<String>,
        theme: Option<Theme>,
    },
    /// Runs this host's daemon, exiting after `idle_exit` seconds without clients when set.
    Daemon { idle_exit: Option<u64> },
    /// Relays stdio to this host's daemon, starting it when none runs.
    Relay,
}

/// Parses `argv` without the program name. Errors are the one-line stderr message, without prefix.
pub fn parse(args: &[String]) -> Result<Command, String> {
    if args.first().is_some_and(|arg| arg == "daemon") {
        return match args[1..].iter().map(String::as_str).collect::<Vec<_>>()[..] {
            [] => Ok(Command::Daemon { idle_exit: None }),
            ["--stdio"] => Ok(Command::Relay),
            ["--idle-exit", seconds] => seconds
                .parse()
                .map(|seconds| Command::Daemon { idle_exit: Some(seconds) })
                .map_err(|_| format!("invalid --idle-exit: {seconds}")),
            ["--idle-exit"] => Err("invalid --idle-exit: ".into()),
            [option, ..] => Err(format!("unexpected daemon argument: {option}")),
        };
    }
    let mut root = None;
    let mut host = None;
    let mut theme = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--theme" {
            theme = Some(match iter.next().map(String::as_str) {
                Some("light") => Theme::Light,
                Some("dark") => Theme::Dark,
                other => return Err(format!("invalid theme: {}", other.unwrap_or(""))),
            });
        } else if arg == "--host" {
            host = Some(iter.next().ok_or("missing --host NAME")?.clone());
        } else if arg.starts_with("--") {
            return Err(format!("unknown option: {arg}"));
        } else if root.is_none() {
            root = Some(arg.as_str());
        } else {
            return Err(format!("unexpected argument: {arg}"));
        }
    }
    let root = match &host {
        Some(_) => match root.map(PathBuf::from) {
            Some(root) if root.is_absolute() => root,
            _ => return Err(format!("with --host, ROOT must be an absolute path: {}", root.unwrap_or(""))),
        },
        None => resolve_root(root.unwrap_or("."))?,
    };
    Ok(Command::Tui { root, host, theme })
}

fn resolve_root(root: &str) -> Result<PathBuf, String> {
    let resolved = std::fs::canonicalize(root).map_err(|_| format!("cannot resolve root: {root}"))?;
    if !resolved.is_dir() {
        return Err(format!("root is not a directory: {root}"));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn tui_defaults_root_to_current_directory() {
        let Command::Tui { root, host, theme } = parse(&[]).unwrap() else {
            panic!("a terminal session")
        };
        assert_eq!(root, std::fs::canonicalize(".").unwrap());
        assert_eq!((host, theme), (None, None));
    }

    #[test]
    fn host_takes_an_absolute_root_as_it_is() {
        assert_eq!(
            parse(&args(&["--host", "box", "/srv/missing-here"])),
            Ok(Command::Tui {
                root: "/srv/missing-here".into(),
                host: Some("box".into()),
                theme: None,
            })
        );
        assert_eq!(
            parse(&args(&["relative", "--host", "box"])).unwrap_err(),
            "with --host, ROOT must be an absolute path: relative"
        );
        assert_eq!(
            parse(&args(&["--host", "box"])).unwrap_err(),
            "with --host, ROOT must be an absolute path: "
        );
        assert_eq!(parse(&args(&["--host"])).unwrap_err(), "missing --host NAME");
    }

    #[test]
    fn daemon_takes_stdio_or_an_idle_exit() {
        assert_eq!(parse(&args(&["daemon"])), Ok(Command::Daemon { idle_exit: None }));
        assert_eq!(parse(&args(&["daemon", "--stdio"])), Ok(Command::Relay));
        assert_eq!(
            parse(&args(&["daemon", "--idle-exit", "300"])),
            Ok(Command::Daemon { idle_exit: Some(300) })
        );
        assert_eq!(parse(&args(&["daemon", "--idle-exit", "x"])).unwrap_err(), "invalid --idle-exit: x");
        assert_eq!(
            parse(&args(&["daemon", "--stdio", "--idle-exit", "1"])).unwrap_err(),
            "unexpected daemon argument: --stdio"
        );
    }

    #[test]
    fn tui_accepts_theme() {
        let parsed = parse(&args(&[".", "--theme", "light"])).unwrap();
        assert!(matches!(
            parsed,
            Command::Tui {
                theme: Some(Theme::Light),
                ..
            }
        ));
        assert_eq!(parse(&args(&["--theme", "blue"])).unwrap_err(), "invalid theme: blue");
    }

    #[test]
    fn vc_8_config_is_an_unknown_option() {
        assert_eq!(parse(&args(&["--config", "x.toml"])).unwrap_err(), "unknown option: --config");
    }

    #[test]
    fn serve_is_a_root_argument() {
        assert_eq!(parse(&args(&["serve"])).unwrap_err(), "cannot resolve root: serve");
    }

    #[test]
    fn tui_rejects_missing_and_file_roots() {
        assert_eq!(
            parse(&args(&["/nonexistent/x"])).unwrap_err(),
            "cannot resolve root: /nonexistent/x"
        );
        let file = std::env::current_exe().unwrap();
        let file = file.to_str().unwrap();
        assert_eq!(parse(&args(&[file])).unwrap_err(), format!("root is not a directory: {file}"));
    }
}
