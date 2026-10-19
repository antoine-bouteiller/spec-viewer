//! Per-project web server ports, kept in `<user config dir>/ports.toml` as `"<project key>" = port`.
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use toml::{Table, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ports {
    file: PathBuf,
}

impl Ports {
    pub fn user() -> Option<Self> {
        super::user_dir().map(|dir| Self::at(dir.join("ports.toml")))
    }
    pub fn at(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into() }
    }
    pub fn file(&self) -> &Path {
        &self.file
    }
    /// A missing or malformed file, or an out-of-range value, means no stored port.
    pub fn get(&self, project: &Path) -> Option<u16> {
        let table = self.read().ok()?;
        let port = table.get(&*project.to_string_lossy())?.as_integer()?;
        u16::try_from(port).ok().filter(|port| *port != 0)
    }
    /// Keeps other projects' entries; refuses to overwrite a file it cannot parse.
    pub fn set(&self, project: &Path, port: u16) -> io::Result<()> {
        let mut table = self.read()?;
        table.insert(project.to_string_lossy().into_owned(), Value::Integer(port.into()));
        if let Some(parent) = self.file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.file, table.to_string())
    }
    fn read(&self) -> io::Result<Table> {
        match fs::read_to_string(&self.file) {
            Ok(source) => source
                .parse::<Table>()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Table::new()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_round_trip_per_project_and_keep_other_entries() {
        let directory = tempfile::tempdir().unwrap();
        let ports = Ports::at(directory.path().join("nested/ports.toml"));
        let (first, second) = (Path::new("/work/a/.git"), Path::new("/work/b/.git"));
        assert_eq!(ports.get(first), None);
        ports.set(first, 41000).unwrap();
        ports.set(second, 42000).unwrap();
        ports.set(first, 41001).unwrap();
        assert_eq!(ports.get(first), Some(41001));
        assert_eq!(ports.get(second), Some(42000));
    }

    #[test]
    fn malformed_file_is_no_port_and_is_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("ports.toml");
        fs::write(&file, "not = [toml").unwrap();
        let ports = Ports::at(&file);
        assert_eq!(ports.get(Path::new("/p")), None);
        assert!(ports.set(Path::new("/p"), 1).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "not = [toml");
        fs::write(&file, "\"/p\" = 70000\n\"/q\" = \"x\"\n").unwrap();
        assert_eq!(ports.get(Path::new("/p")), None);
        assert_eq!(ports.get(Path::new("/q")), None);
    }
}
