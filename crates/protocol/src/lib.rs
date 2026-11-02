//! The daemon protocol: messages, line framing, the runtime directory, and the blocking client.
mod client;
mod messages;
mod runtime;

pub use client::{Client, Event, Watch};
pub use messages::*;
pub use runtime::Runtime;

pub use spec_viewer_config::{Layer, StatusConfig};
pub use spec_viewer_core::{
    Align, Block, Chip, ChipValue, CodeTab, ComparisonOption, Component, ComponentBody, Corpus, Degraded, DocNode, Document, Hit, Inline,
    ListItem, MarginNote, Meta, NodeDetail, Opened, Project, Status, Step, TabDocs, TocEntry, Worktree, search,
};

use std::io::{self, BufRead, Write};

/// Writes `message` as one JSON line.
pub fn send(writer: &mut impl Write, message: &impl serde::Serialize) -> io::Result<()> {
    let mut line = serde_json::to_vec(message).map_err(io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}

/// Reads the next JSON line; `None` at the end of the stream.
pub fn receive<T: serde::de::DeserializeOwned>(reader: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    serde_json::from_str(&line)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
