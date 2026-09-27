use crate::remote::Location;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Everything the app remembers between runs, kept in one JSON file.
/// Never a password or anything read from the database.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct State {
    /// Owned by the frontend; the backend stores it as-is.
    pub settings: Map<String, Value>,
    /// The database opened last; with `remote`, its working copy.
    pub database: Option<PathBuf>,
    /// Where the database is synced to, if anywhere.
    pub remote: Option<Remote>,
    /// The key file used with it, if any.
    pub key_file: Option<PathBuf>,
    pub window: Option<WindowGeometry>,
}

/// A remote file the working copy is synced with, and where the last sync
/// left them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    pub location: Location,
    /// The remote file's revision at the last sync; `None` before the first
    /// upload, or when the file was gone.
    pub revision: Option<String>,
    /// The working copy's hash (hex SHA-256) at the last sync: when the
    /// file's hash differs, it changed since.
    pub synced: Option<String>,
}

/// Window placement in logical pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub maximized: bool,
}

pub struct Store {
    path: PathBuf,
    state: Mutex<State>,
}

impl Store {
    /// Loads the state file. A missing file gives an empty state; a corrupt
    /// one is moved aside to `<name>.bak` rather than silently overwritten.
    pub fn load(path: PathBuf) -> Self {
        let state = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| {
                let _ = fs::rename(&path, path.with_extension("json.bak"));
                State::default()
            }),
            Err(_) => State::default(),
        };
        Store { path, state: Mutex::new(state) }
    }

    /// The folder the state file lives in; the icon cache sits beside it.
    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }

    pub fn read<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        f(&self.state.lock().unwrap())
    }

    /// Applies a change and writes the whole state to disk.
    pub fn update(&self, f: impl FnOnce(&mut State)) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        f(&mut state);
        write_atomically(&self.path, &serde_json::to_vec_pretty(&*state)?)
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_empty_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::load(dir.path().join("pswm.json"));
        assert_eq!(store.read(State::clone), State::default());
    }

    #[test]
    fn updates_survive_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("pswm.json");
        let store = Store::load(path.clone());
        store.update(|s| s.database = Some(PathBuf::from(r"C:\Vault\base.kdbx"))).unwrap();
        let reloaded = Store::load(path);
        assert_eq!(reloaded.read(|s| s.database.clone()), Some(PathBuf::from(r"C:\Vault\base.kdbx")));
        assert_eq!(reloaded.dir(), dir.path().join("nested"));
    }

    #[test]
    fn corrupt_file_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pswm.json");
        fs::write(&path, "{ not json").unwrap();
        let store = Store::load(path.clone());
        assert_eq!(store.read(State::clone), State::default());
        assert_eq!(fs::read_to_string(path.with_extension("json.bak")).unwrap(), "{ not json");
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pswm.json");
        fs::write(&path, r#"{"settings":{"clearClipboard":30},"future":1}"#).unwrap();
        let store = Store::load(path);
        assert_eq!(store.read(|s| s.settings["clearClipboard"].clone()), 30);
    }
}
