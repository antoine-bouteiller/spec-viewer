mod app;
mod navigation;
mod theme;

pub use app::{App, Focus, Hint, RailEntry, Tab};

use std::{
    io::{self, Write},
    path::Path,
    process::ExitCode,
    time::Duration,
};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, MouseEventKind},
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

pub fn run(root: &Path, theme: Option<Theme>) -> ExitCode {
    match start(root, theme) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("plan-viewer: {error}");
            ExitCode::from(1)
        }
    }
}

fn start(root: &Path, theme: Option<Theme>) -> io::Result<()> {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |information| {
        restore();
        previous_hook(information);
    }));
    enable_raw_mode()?;
    let _restore = Restore;
    let theme = match theme {
        Some(theme) => (Some(theme), Vec::new()),
        None => theme::detect(),
    };
    let mut app = App::new(root, theme.0.unwrap_or(Theme::Dark), io::stdout())?;
    for character in String::from_utf8_lossy(&theme.1).chars() {
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
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => app.scroll_by(-3),
                    MouseEventKind::ScrollDown => app.scroll_by(3),
                    _ => {}
                },
                _ => {}
            }
        }
    }
    io::stdout().flush()
}
