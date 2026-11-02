pub mod fixture;

use std::path::Path;

use spec_viewer_daemon::{Daemon, Options};
use spec_viewer_protocol::{Client, Runtime};

/// A client of an in-process daemon whose user config is `user`, if any.
pub fn client(user: Option<&Path>) -> Client {
    Daemon::new(&Options {
        runtime: Runtime::at("/nonexistent"),
        user_config: user.map(Path::to_path_buf),
        known: None,
        idle_exit: None,
    })
    .unwrap()
    .connect()
    .unwrap()
}
