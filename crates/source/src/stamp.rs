use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub mtime: SystemTime,
    pub size: u64,
}
impl From<&std::fs::Metadata> for Stamp {
    fn from(meta: &std::fs::Metadata) -> Self {
        Self {
            mtime: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            size: meta.len(),
        }
    }
}
