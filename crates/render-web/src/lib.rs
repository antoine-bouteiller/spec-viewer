//! The browser surface: semantic documents as HTML, and a loopback HTTP server for the web client.

mod highlight;
pub mod html;
mod server;

pub use server::start;
