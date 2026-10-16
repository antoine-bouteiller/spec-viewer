use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Number, Value};

const STALE_AFTER: Duration = Duration::from_secs(10);
const TIMEOUT: Duration = Duration::from_secs(15);
const RETRY: Duration = Duration::from_millis(25);

#[derive(Clone, Debug, Serialize)]
pub(super) struct Entry {
    pub holders: Vec<Number>,
    pub port: Number,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Entry {
    pub fn stopped(port: Number) -> Self {
        Self {
            holders: Vec::new(),
            port,
            pid: None,
            url: None,
        }
    }

    pub fn owned(&self) -> bool {
        self.pid.as_ref().and_then(Number::as_u64) == Some(u64::from(std::process::id()))
    }
}

pub(super) fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("PLAN_VIEWER_CONFIG").filter(|path| !path.is_empty()) {
        return path.into();
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("plan-viewer/viewers.json")
}

fn alive(pid: &Number) -> io::Result<bool> {
    let number = pid.as_f64().expect("JSON number");
    if number < f64::from(i32::MIN) || number > f64::from(i32::MAX) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "pid out of range"));
    }
    let result = unsafe { libc::kill(number as libc::pid_t, 0) };
    if result == 0 {
        return Ok(true);
    }
    match io::Error::last_os_error().raw_os_error() {
        Some(libc::EPERM) => Ok(true),
        Some(libc::ESRCH) => Ok(false),
        _ => Err(io::Error::last_os_error()),
    }
}

fn prune(value: Value) -> io::Result<Option<Entry>> {
    let Some(object) = value.as_object() else { return Ok(None) };
    let (Some(port), Some(holders)) = (
        object.get("port").and_then(Value::as_number),
        object.get("holders").and_then(Value::as_array),
    ) else {
        return Ok(None);
    };
    let mut live = Vec::new();
    for holder in holders.iter().filter_map(Value::as_number) {
        if alive(holder)? {
            live.push(holder.clone());
        }
    }
    let pid = object.get("pid").and_then(Value::as_number).cloned();
    if let Some(pid) = &pid
        && !alive(pid)?
    {
        return Ok(Some(Entry::stopped(port.clone())));
    }
    Ok(Some(Entry {
        holders: live,
        port: port.clone(),
        pid,
        url: object.get("url").and_then(Value::as_str).map(str::to_owned),
    }))
}

#[derive(Deserialize)]
struct RawConfig {
    viewers: RawViewers,
}

struct RawViewers(Vec<(String, Value)>);

impl<'de> Deserialize<'de> for RawViewers {
    fn deserialize<Decoder: Deserializer<'de>>(decoder: Decoder) -> Result<Self, Decoder::Error> {
        struct Entries;
        impl<'de> Visitor<'de> for Entries {
            type Value = RawViewers;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a viewers object")
            }
            fn visit_map<Access: MapAccess<'de>>(self, mut access: Access) -> Result<Self::Value, Access::Error> {
                let mut entries: Vec<(String, Value)> = Vec::new();
                while let Some((key, value)) = access.next_entry()? {
                    if let Some((_, previous)) = entries.iter_mut().find(|(existing, _)| existing == &key) {
                        *previous = value;
                    } else {
                        entries.push((key, value));
                    }
                }
                Ok(RawViewers(entries))
            }
        }
        decoder.deserialize_map(Entries)
    }
}

fn read_config(path: &Path) -> io::Result<Vec<(String, Entry)>> {
    let source = match fs::read(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let Ok(config) = serde_json::from_slice::<RawConfig>(&source) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for (key, value) in config.viewers.0 {
        if let Some(entry) = prune(value)? {
            entries.push((key, entry));
        }
    }
    Ok(entries)
}

pub(super) fn read_entry(path: &Path, key: &str) -> io::Result<Option<Entry>> {
    Ok(read_config(path)?
        .into_iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, entry)| entry))
}

fn suffix(path: &Path, ending: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(ending);
    name.into()
}

fn create_exclusive(path: &Path) -> io::Result<bool> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

fn stale(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > STALE_AFTER)
}

fn remove(path: &Path, recursive: bool) -> io::Result<()> {
    let result = if recursive && path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match result {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

struct Lock(Option<PathBuf>);

impl Lock {
    fn release(mut self) -> io::Result<()> {
        remove(&self.0.take().expect("owned lock"), false)
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = remove(&path, false);
        }
    }
}

fn reap(lock: &Path) -> io::Result<()> {
    let guard = suffix(lock, ".reap");
    if !create_exclusive(&guard)? {
        if stale(&guard) {
            remove(&guard, false)?;
        }
        return Ok(());
    }
    let release = Lock(Some(guard));
    let result = if stale(lock) { remove(lock, true) } else { Ok(()) };
    release.release()?;
    result
}

fn acquire(path: &Path) -> io::Result<Lock> {
    fs::create_dir_all(
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    let lock = suffix(path, ".lock");
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if create_exclusive(&lock)? {
            return Ok(Lock(Some(lock)));
        }
        if stale(&lock) {
            reap(&lock)?;
        } else if Instant::now() > deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("Timed out waiting for {}", lock.display()),
            ));
        }
        std::thread::sleep(RETRY);
    }
}

fn uuid() -> io::Result<String> {
    let mut bytes = [0_u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

pub(super) fn update_entry(path: &Path, key: &str, mutate: impl FnOnce(Option<Entry>) -> Option<Entry>) -> io::Result<Option<Entry>> {
    let lock = acquire(path)?;
    let result = (|| {
        let mut config = read_config(path)?;
        let position = config.iter().position(|(candidate, _)| candidate == key);
        let entry = mutate(position.map(|position| config[position].1.clone()));
        match (position, &entry) {
            (Some(position), Some(entry)) => config[position].1 = entry.clone(),
            (Some(position), None) => {
                config.remove(position);
            }
            (None, Some(entry)) => config.push((key.to_owned(), entry.clone())),
            (None, None) => {}
        }
        let entries = config
            .iter()
            .map(|(key, entry)| Ok(format!("{}:{}", serde_json::to_string(key)?, serde_json::to_string(entry)?)))
            .collect::<Result<Vec<_>, serde_json::Error>>()?;
        let temporary = suffix(path, &format!(".{}.{}.tmp", std::process::id(), uuid()?));
        let write = fs::write(&temporary, format!("{{\"viewers\":{{{}}}}}", entries.join(","))).and_then(|()| fs::rename(&temporary, path));
        if write.is_err() {
            remove(&temporary, false)?;
        }
        write?;
        Ok(entry)
    })();
    lock.release()?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn stopped(port: u16) -> Option<Entry> {
        Some(Entry::stopped(Number::from(port)))
    }

    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    fn age(path: &Path) {
        File::open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(11)))
            .unwrap();
    }

    #[test]
    fn pruning_dead_pids_preserves_ports_and_live_holders() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("viewers.json");
        let pid = std::process::id();
        let dead = dead_pid();
        fs::write(&path, format!("{{\"viewers\":{{\"live\":{{\"port\":42,\"pid\":{pid},\"holders\":[{dead},\"bad\",{pid}],\"url\":\"http://localhost:42\"}},\"dead\":{{\"port\":43,\"pid\":{dead},\"holders\":[{pid}],\"url\":\"old\"}},\"invalid\":{{\"port\":\"44\",\"holders\":[]}}}}}}")).unwrap();
        let live = read_entry(&path, "live").unwrap().unwrap();
        assert_eq!(live.holders, [Number::from(pid)]);
        assert!(live.owned());
        assert_eq!(live.url.as_deref(), Some("http://localhost:42"));
        let dead = read_entry(&path, "dead").unwrap().unwrap();
        assert_eq!(serde_json::to_value(dead).unwrap(), serde_json::json!({"holders": [], "port": 43}));
        assert!(read_entry(&path, "invalid").unwrap().is_none());
        update_entry(&path, "live", std::convert::identity).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("invalid"));
    }

    #[test]
    fn malformed_json_and_invalid_config_shapes_are_empty() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("viewers.json");
        for source in [
            "{",
            "null",
            "[]",
            "{}",
            "{\"viewers\":[]}",
            "{\"viewers\":null}",
            "{\"viewers\":{\"bad\":false}}",
        ] {
            fs::write(&path, source).unwrap();
            assert!(read_config(&path).unwrap().is_empty(), "{source}");
        }
        fs::remove_file(&path).unwrap();
        assert!(read_config(&path).unwrap().is_empty());
        fs::create_dir(&path).unwrap();
        assert!(read_config(&path).is_err());
    }

    #[test]
    fn updates_preserve_bun_compact_field_and_project_insertion_order() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("viewers.json");
        fs::write(
            &path,
            "{\"viewers\":{\"zeta\":{\"port\":12,\"holders\":[]},\"alpha\":{\"holders\":[],\"port\":13}}}",
        )
        .unwrap();
        update_entry(&path, "alpha", |entry| {
            let mut entry = entry.unwrap();
            entry.pid = Some(Number::from(std::process::id()));
            entry.url = Some("http://127.0.0.1:13".into());
            Some(entry)
        })
        .unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!(
                "{{\"viewers\":{{\"zeta\":{{\"holders\":[],\"port\":12}},\"alpha\":{{\"holders\":[],\"port\":13,\"pid\":{},\"url\":\"http://127.0.0.1:13\"}}}}}}",
                std::process::id()
            )
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn contending_updates_wait_and_do_not_lose_either_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/viewers.json");
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let first_path = path.clone();
        let first = std::thread::spawn(move || {
            update_entry(&first_path, "first", |_| {
                entered_sender.send(()).unwrap();
                release_receiver.recv().unwrap();
                stopped(1)
            })
            .unwrap()
        });
        entered_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        let (finished_sender, finished_receiver) = mpsc::channel();
        let second_path = path.clone();
        let second = std::thread::spawn(move || {
            update_entry(&second_path, "second", |_| stopped(2)).unwrap();
            finished_sender.send(()).unwrap();
        });
        assert!(finished_receiver.recv_timeout(Duration::from_millis(150)).is_err());
        release_sender.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();
        assert_eq!(read_entry(&path, "first").unwrap().unwrap().port, Number::from(1));
        assert_eq!(read_entry(&path, "second").unwrap().unwrap().port, Number::from(2));
        assert!(!suffix(&path, ".lock").exists());
    }

    #[test]
    fn stale_file_and_legacy_directory_locks_are_reaped() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("viewers.json");
        let lock = suffix(&path, ".lock");
        for legacy_directory in [false, true] {
            if legacy_directory {
                fs::create_dir(&lock).unwrap();
            } else {
                fs::write(&lock, "").unwrap();
            }
            age(&lock);
            update_entry(&path, "project", |_| stopped(5)).unwrap();
            assert_eq!(read_entry(&path, "project").unwrap().unwrap().port, Number::from(5));
            assert!(!lock.exists());
            assert!(!suffix(&lock, ".reap").exists());
        }
    }

    #[test]
    fn stale_reap_guard_is_removed_without_deleting_a_fresh_lock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("viewers.json");
        let lock = suffix(&path, ".lock");
        let guard = suffix(&lock, ".reap");
        fs::write(&lock, "").unwrap();
        age(&lock);
        fs::write(&guard, "").unwrap();
        age(&guard);
        update_entry(&path, "project", |_| stopped(7)).unwrap();
        assert!(!guard.exists());
        fs::write(&lock, "fresh").unwrap();
        reap(&lock).unwrap();
        assert_eq!(fs::read_to_string(&lock).unwrap(), "fresh");
    }
}
