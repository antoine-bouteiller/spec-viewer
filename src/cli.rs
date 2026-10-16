use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Tui { root: PathBuf, theme: Option<Theme> },
    Serve { root: PathBuf, port: u16 },
}

/// Parses `argv` without the program name. Errors are the one-line stderr message, without prefix.
pub fn parse(args: &[String]) -> Result<Command, String> {
    match args.first().map(String::as_str) {
        Some("serve") => parse_serve(&args[1..]),
        _ => parse_tui(args),
    }
}

fn parse_serve(args: &[String]) -> Result<Command, String> {
    let Some(root) = args.first().filter(|arg| !arg.starts_with("--")) else {
        return Err("root directory is required".into());
    };
    let rest = &args[1..];
    let port = match rest.iter().position(|arg| arg == "--port") {
        None => 0,
        Some(index) => {
            let value = rest.get(index + 1).ok_or("--port requires a value")?;
            value.parse::<u16>().map_err(|_| format!("invalid port: {value}"))?
        }
    };
    Ok(Command::Serve {
        root: resolve_root(root)?,
        port,
    })
}

fn parse_tui(args: &[String]) -> Result<Command, String> {
    let mut root = None;
    let mut theme = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--theme" {
            theme = Some(match iter.next().map(String::as_str) {
                Some("light") => Theme::Light,
                Some("dark") => Theme::Dark,
                other => return Err(format!("invalid theme: {}", other.unwrap_or(""))),
            });
        } else if root.is_none() {
            root = Some(arg.as_str());
        } else {
            return Err(format!("unexpected argument: {arg}"));
        }
    }
    Ok(Command::Tui {
        root: resolve_root(root.unwrap_or("."))?,
        theme,
    })
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
        let Command::Tui { root, theme } = parse(&[]).unwrap() else {
            panic!()
        };
        assert_eq!(root, std::fs::canonicalize(".").unwrap());
        assert_eq!(theme, None);
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
    fn serve_requires_root() {
        assert_eq!(parse(&args(&["serve"])).unwrap_err(), "root directory is required");
    }

    #[test]
    fn serve_rejects_missing_and_file_roots() {
        assert_eq!(
            parse(&args(&["serve", "/nonexistent/x"])).unwrap_err(),
            "cannot resolve root: /nonexistent/x"
        );
        let file = std::env::current_exe().unwrap();
        let file = file.to_str().unwrap();
        assert_eq!(
            parse(&args(&["serve", file])).unwrap_err(),
            format!("root is not a directory: {file}")
        );
    }

    #[test]
    fn serve_parses_port() {
        assert_eq!(
            parse(&args(&["serve", ".", "--port", "8080"])).unwrap(),
            Command::Serve {
                root: std::fs::canonicalize(".").unwrap(),
                port: 8080
            }
        );
        assert_eq!(parse(&args(&["serve", ".", "--port"])).unwrap_err(), "--port requires a value");
        assert_eq!(parse(&args(&["serve", ".", "--port", "70000"])).unwrap_err(), "invalid port: 70000");
        assert_eq!(parse(&args(&["serve", ".", "--port", "-1"])).unwrap_err(), "invalid port: -1");
    }
}
