use std::path::PathBuf;

use spec_viewer_config::Mode as Theme;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Tui {
        root: PathBuf,
        theme: Option<Theme>,
        config: Option<PathBuf>,
    },
}

/// Parses `argv` without the program name. Errors are the one-line stderr message, without prefix.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let mut root = None;
    let mut theme = None;
    let mut config = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--theme" {
            theme = Some(match iter.next().map(String::as_str) {
                Some("light") => Theme::Light,
                Some("dark") => Theme::Dark,
                other => return Err(format!("invalid theme: {}", other.unwrap_or(""))),
            });
        } else if arg == "--config" {
            config = Some(PathBuf::from(iter.next().ok_or("missing --config PATH")?));
        } else if root.is_none() {
            root = Some(arg.as_str());
        } else {
            return Err(format!("unexpected argument: {arg}"));
        }
    }
    Ok(Command::Tui {
        root: resolve_root(root.unwrap_or("."))?,
        theme,
        config,
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
        let Command::Tui { root, theme, config } = parse(&[]).unwrap();
        assert_eq!(root, std::fs::canonicalize(".").unwrap());
        assert_eq!(theme, None);
        assert_eq!(config, None);
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
