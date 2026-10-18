//! Terminal rendering and interactive document navigation.

pub mod highlight;
pub mod terminal;
pub mod theme;
pub mod tui;
pub use theme::Theme;

pub use tui::run;
