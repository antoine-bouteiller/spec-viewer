mod app;
mod navigation;
mod theme;

pub use app::{App, Focus, Hint, RailEntry, Search, Tab};

use std::{
    io::{self, Write},
    path::Path,
    process::ExitCode,
    sync::atomic::{AtomicBool, Ordering},
    sync::mpsc::TryRecvError,
    time::Duration,
};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute, queue,
    style::Print,
    terminal::{
        BeginSynchronizedUpdate, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen, SetTitle, disable_raw_mode,
        enable_raw_mode, supports_keyboard_enhancement,
    },
};
use ratatui::{Terminal, backend::Backend, backend::CrosstermBackend, layout::Rect};

use crate::terminal::Theme;
use spec_viewer_config::Keys;
use spec_viewer_protocol::{Client, Error};

struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        restore();
    }
}

/// Whether the kitty keyboard flags were pushed and must be popped.
static KEYBOARD: AtomicBool = AtomicBool::new(false);
/// XTWINOPS: save and restore the terminal's title around the session.
const PUSH_TITLE: &str = "\x1b[22;0t";
const POP_TITLE: &str = "\x1b[23;0t";

fn restore() {
    let _ = disable_raw_mode();
    if KEYBOARD.swap(false, Ordering::SeqCst) {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        io::stdout(),
        DisableMouseCapture,
        LeaveAlternateScreen,
        Print(POP_TITLE),
        crossterm::cursor::Show
    );
}

/// A session on the worktree containing `root`, absolute on `host`. `diff` is the initial state of
/// the `Diff` toggle.
pub fn run(client: Client, host: Option<&str>, root: &Path, theme: Theme, keys: Keys, diff: bool) -> ExitCode {
    match start(client, host, root, theme, keys, diff) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("spec-viewer: {error}");
            ExitCode::from(1)
        }
    }
}

fn start(client: Client, host: Option<&str>, root: &Path, theme: Theme, keys: Keys, diff: bool) -> io::Result<()> {
    // Report an unknown root or a broken config before the terminal changes mode.
    client.resolve(host, root).map_err(|error| match (error, host) {
        (Error::UnknownHost, Some(host)) => io::Error::other(format!("unknown host: {host}")),
        (error, _) => io::Error::other(error),
    })?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |information| {
        restore();
        previous_hook(information);
    }));
    enable_raw_mode()?;
    let _restore = Restore;
    let (detected, input) = if theme.is_auto() { theme::detect() } else { (None, Vec::new()) };
    let web_url = client.web_url().map(str::to_owned);
    let mut app = App::new(client, host, root, theme.resolve_auto(detected), io::stdout())?;
    app.web_url = web_url;
    app.keys = keys;
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
    // Reports Cmd (super) and other modifiers that legacy encodings drop.
    if supports_keyboard_enhancement().unwrap_or(false) {
        execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        KEYBOARD.store(true, Ordering::SeqCst);
    }
    execute!(io::stdout(), Print(PUSH_TITLE), EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut title = String::new();
    let mut linked: Option<Rect> = None;
    let mut watched = None;
    let mut watcher = None;
    while !app.quit {
        let current = (app.worktree_host().map(str::to_owned), app.worktree_path().to_path_buf());
        if watched.as_ref() != Some(&current) {
            watched = Some(current);
            watcher = match app.watch() {
                Ok(watch) => Some(watch),
                Err(error) => {
                    app.message = Some(format!("Live reload is off: {error}"));
                    None
                }
            };
        }
        let mut reload = false;
        while let Some(watch) = &watcher {
            match watch.try_recv() {
                Ok(_) => reload = true,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    app.message = Some("Lost the connection to the daemon; live reload is off.".into());
                    watcher = None;
                }
            }
        }
        if reload {
            app.reload();
        }
        if app.title() != title {
            title = app.title();
            execute!(io::stdout(), SetTitle(&title))?;
        }
        queue!(io::stdout(), BeginSynchronizedUpdate)?;
        let mut link = Vec::new();
        terminal.draw(|frame| {
            app.draw(frame);
            link = app.link.map(|area| link_cells(frame.buffer_mut(), area)).unwrap_or_default();
        })?;
        // A cell keeps its hyperlink until rewritten; redraw everything when the link moves or goes.
        if linked.is_some() && linked != app.link {
            terminal.clear()?;
            terminal.draw(|frame| app.draw(frame))?;
        }
        linked = app.link;
        if let (Some(area), Some(target)) = (app.link, app.web_link()) {
            queue!(io::stdout(), Print(format!("\x1b]8;;{target}\x1b\\")))?;
            terminal
                .backend_mut()
                .draw(link.iter().enumerate().map(|(index, cell)| (area.x + index as u16, area.y, cell)))?;
            queue!(io::stdout(), Print("\x1b]8;;\x1b\\"))?;
        }
        execute!(io::stdout(), EndSynchronizedUpdate)?;
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

fn link_cells(buffer: &ratatui::buffer::Buffer, area: Rect) -> Vec<ratatui::buffer::Cell> {
    (area.left()..area.right()).map(|x| buffer[(x, area.y)].clone()).collect()
}
