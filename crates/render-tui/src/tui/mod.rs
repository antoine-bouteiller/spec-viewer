mod app;
mod navigation;
mod theme;

pub use app::{App, Focus, Hint, RailEntry, Tab};

use std::{
    io::{self, Write},
    process::ExitCode,
    time::Duration,
};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::terminal::Theme;

struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        restore();
    }
}

fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen, crossterm::cursor::Show);
}

/// `diff` is the initial state of the `Diff` toggle.
pub fn run(library: spec_viewer_core::Library, theme: Theme, web_url: Option<String>, diff: bool) -> ExitCode {
    match start(library, theme, web_url, diff) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("spec-viewer: {error}");
            ExitCode::from(1)
        }
    }
}

fn start(library: spec_viewer_core::Library, theme: Theme, web_url: Option<String>, diff: bool) -> io::Result<()> {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |information| {
        restore();
        previous_hook(information);
    }));
    enable_raw_mode()?;
    let _restore = Restore;
    let (detected, input) = if theme.is_auto() { theme::detect() } else { (None, Vec::new()) };
    let mut app = App::new(library, theme.resolve_auto(detected), io::stdout())?;
    app.web_url = web_url;
    app.diff = diff;
    for character in String::from_utf8_lossy(&input).chars() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let key = match character {
            '\x03' => KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            '\n' | '\r' => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            '\t' => KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            _ => KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
        };
        app.handle_key(key);
    }
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut watched_path = app.worktree_path().to_path_buf();
    let mut watcher = app.watch()?;
    while !app.quit {
        if app.worktree_path() != watched_path {
            watched_path = app.worktree_path().to_path_buf();
            watcher = app.watch()?;
        }
        let mut reload = false;
        while watcher.reloads.try_recv().is_ok() {
            reload = true;
        }
        if reload {
            app.reload();
        }
        terminal.draw(|frame| app.draw(frame))?;
        if event::poll(Duration::from_millis(25))? {
            match event::read()? {
                Event::Key(key) => app.handle_key(key),
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                _ => {}
            }
        }
    }
    io::stdout().flush()
}
