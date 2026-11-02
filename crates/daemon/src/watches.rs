//! One debounced watch per worktree, shared by every subscriber and dropped with the last one.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
};

use spec_viewer_protocol::{DaemonMessage, Error};
use spec_viewer_source::{Watcher, watch_with};

use crate::{Connection, Daemon};

#[derive(Default)]
pub(crate) struct Watches {
    worktrees: Mutex<HashMap<PathBuf, Arc<Shared>>>,
}

#[derive(Default)]
struct Shared {
    subscribers: Arc<Mutex<Subscribers>>,
    watcher: Mutex<Option<Watcher>>,
}

#[derive(Default)]
struct Subscribers {
    next: u64,
    entries: HashMap<u64, (Weak<Connection>, u64)>,
}

/// Unsubscribes when dropped.
pub(crate) struct Subscription {
    daemon: Weak<Daemon>,
    worktree: PathBuf,
    shared: Arc<Shared>,
    key: u64,
}

impl Watches {
    /// Sends `Changed { id }` on `connection` per burst of changes in `worktree`.
    pub(crate) fn subscribe(
        &self,
        daemon: &Arc<Daemon>,
        connection: &Arc<Connection>,
        id: u64,
        worktree: PathBuf,
    ) -> Result<Subscription, Error> {
        let mut worktrees = self.worktrees.lock().expect("watches");
        let shared = Arc::clone(worktrees.entry(worktree.clone()).or_default());
        let key = {
            let mut subscribers = shared.subscribers.lock().expect("subscribers");
            subscribers.next += 1;
            let key = subscribers.next;
            subscribers.entries.insert(key, (Arc::downgrade(connection), id));
            key
        };
        let subscription = Subscription {
            daemon: Arc::downgrade(daemon),
            worktree: worktree.clone(),
            shared: Arc::clone(&shared),
            key,
        };
        let mut watcher = shared.watcher.lock().expect("watcher");
        if watcher.is_none() {
            // The tabs to watch are those of every subscriber's effective config, read per burst.
            let (configs, notify) = (Arc::clone(&shared.subscribers), Arc::clone(&shared.subscribers));
            let (weak, root) = (Arc::downgrade(daemon), worktree.clone());
            let started = watch_with(
                &worktree,
                move || {
                    let Some(daemon) = weak.upgrade() else { return vec![] };
                    connections(&configs)
                        .iter()
                        .filter_map(|(connection, _)| daemon.parse_config(connection, &root).ok())
                        .collect()
                },
                move || {
                    for (connection, id) in connections(&notify) {
                        connection.send(&DaemonMessage::Changed { id });
                    }
                    true
                },
            )
            .map_err(|error| Error::unreachable(format!("cannot watch {}: {error}", worktree.display())));
            match started {
                Ok(started) => *watcher = Some(started),
                Err(error) => {
                    // The subscription's drop takes the map lock.
                    drop(watcher);
                    drop(worktrees);
                    drop(subscription);
                    return Err(error);
                }
            }
        }
        drop(watcher);
        drop(worktrees);
        // A new subscriber's config may list folders the others do not.
        shared.refresh();
        Ok(subscription)
    }

    /// Re-reads every watch's tab folders after a layer change, which makes no event in a worktree.
    pub(crate) fn refresh(&self) {
        let shared: Vec<_> = self.worktrees.lock().expect("watches").values().cloned().collect();
        for shared in shared {
            shared.refresh();
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.worktrees.lock().expect("watches").len()
    }
}

impl Shared {
    fn refresh(&self) {
        if let Some(watcher) = self.watcher.lock().expect("watcher").as_ref() {
            watcher.refresh();
        }
    }
}

fn connections(subscribers: &Mutex<Subscribers>) -> Vec<(Arc<Connection>, u64)> {
    let subscribers = subscribers.lock().expect("subscribers");
    subscribers
        .entries
        .values()
        .filter_map(|(connection, id)| Some((connection.upgrade()?, *id)))
        .collect()
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let empty = {
            let mut subscribers = self.shared.subscribers.lock().expect("subscribers");
            subscribers.entries.remove(&self.key);
            subscribers.entries.is_empty()
        };
        if empty && let Some(daemon) = self.daemon.upgrade() {
            let mut worktrees = daemon.watches.worktrees.lock().expect("watches");
            let still_empty = self.shared.subscribers.lock().expect("subscribers").entries.is_empty();
            if still_empty
                && worktrees
                    .get(&self.worktree)
                    .is_some_and(|shared| Arc::ptr_eq(shared, &self.shared))
            {
                worktrees.remove(&self.worktree);
            }
        }
    }
}
