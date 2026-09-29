use crate::dbfile::{replace_file, sibling};
use crate::remote::Location;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Everything the app remembers between runs, kept in one JSON file.
/// Never a password or a secret; of the database, only its name and
/// description, for the unlock screen.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct State {
    /// Owned by the frontend; the backend stores it as-is.
    pub settings: Map<String, Value>,
    /// The databases the app knows.
    pub databases: Vec<Known>,
    /// The file of the one the unlock screen opens.
    pub current: Option<PathBuf>,
    pub window: Option<WindowGeometry>,
    /// The one database of an older state file, read into `databases` once.
    #[serde(skip_serializing, rename = "database")]
    old_database: Option<PathBuf>,
    #[serde(skip_serializing, rename = "remote")]
    old_remote: Option<Remote>,
    #[serde(skip_serializing, rename = "keyFile")]
    old_key_file: Option<PathBuf>,
}

/// A database in the list: its file, the key file used with it, the remote
/// file it is synced with, and its name and description as last unlocked
/// (the file is encrypted: the unlock screen cannot read them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Known {
    pub file: PathBuf,
    pub key_file: Option<PathBuf>,
    pub remote: Option<Remote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Known {
    fn new(file: PathBuf, key_file: Option<PathBuf>, remote: Option<Remote>) -> Self {
        Known { file, key_file, remote, name: None, description: None }
    }

    /// The database's name, or its file's when it has none.
    pub fn title(&self) -> String {
        self.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| crate::remote::file_name(&self.file))
    }
}

impl State {
    /// The database the unlock screen opens.
    pub fn current(&self) -> Option<&Known> {
        let file = self.current.as_ref()?;
        self.databases.iter().find(|d| &d.file == file)
    }

    /// Where the current database is synced to, if anywhere.
    pub fn remote(&self) -> Option<&Remote> {
        self.current()?.remote.as_ref()
    }

    pub fn current_mut(&mut self) -> Option<&mut Known> {
        let file = self.current.clone()?;
        self.databases.iter_mut().find(|d| d.file == file)
    }

    /// Makes `file` the current database, adding it (without sync) if the
    /// list does not have it yet.
    pub fn select(&mut self, file: PathBuf) -> &mut Known {
        if !self.databases.iter().any(|d| d.file == file) {
            self.databases.push(Known::new(file.clone(), None, None));
        }
        self.current = Some(file.clone());
        self.databases.iter_mut().find(|d| d.file == file).expect("just added")
    }

    /// Forgets `file`; the next database in the list becomes current.
    pub fn remove(&mut self, file: &Path) {
        self.databases.retain(|d| d.file != file);
        if self.current.as_deref() == Some(file) {
            self.current = self.databases.first().map(|d| d.file.clone());
        }
    }

    /// An older state file kept one database: it becomes a list of one.
    fn upgrade(&mut self) {
        if let Some(file) = self.old_database.take() {
            if self.databases.is_empty() {
                let (key_file, remote) = (self.old_key_file.take(), self.old_remote.take());
                self.databases.push(Known::new(file.clone(), key_file, remote));
                self.current = Some(file);
            }
        }
        self.old_remote = None;
        self.old_key_file = None;
    }
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
            Ok(text) => serde_json::from_str(&text)
                .map(|mut state: State| {
                    state.upgrade();
                    state
                })
                .unwrap_or_else(|_| {
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

/// Creates the folder if needed, and replaces the file whole.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    replace_file(path, &sibling(path, ".tmp"), bytes)
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
        store.update(|s| s.select(PathBuf::from(r"C:\Vault\base.kdbx")).key_file = Some("k.key".into())).unwrap();
        let reloaded = Store::load(path);
        let current = reloaded.read(|s| s.current().cloned()).unwrap();
        assert_eq!(current.file, PathBuf::from(r"C:\Vault\base.kdbx"));
        assert_eq!(current.key_file, Some("k.key".into()));
        assert_eq!(reloaded.dir(), dir.path().join("nested"));
    }

    #[test]
    fn an_older_state_file_becomes_a_list_of_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pswm.json");
        fs::write(&path, r#"{"database":"C:\\a.kdbx","keyFile":"C:\\a.key","remote":{"location":{"kind":"folder","path":"D:\\a.kdbx"},"revision":"r","synced":"h"}}"#).unwrap();
        let store = Store::load(path.clone());
        let state = store.read(State::clone);
        assert_eq!(state.databases.len(), 1);
        let current = state.current().unwrap();
        assert_eq!((current.file.to_str(), current.key_file.as_deref().and_then(Path::to_str)), (Some(r"C:\a.kdbx"), Some(r"C:\a.key")));
        assert_eq!(current.remote.as_ref().unwrap().revision.as_deref(), Some("r"));
        // Written back in the new shape only.
        store.update(|_| {}).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("databases") && !text.contains("\"database\""), "{text}");
    }

    #[test]
    fn selecting_adds_and_removing_moves_on() {
        let mut state = State::default();
        state.select("a.kdbx".into());
        state.select("b.kdbx".into());
        state.select("a.kdbx".into());
        assert_eq!(state.databases.len(), 2);
        assert_eq!(state.current.as_deref(), Some(Path::new("a.kdbx")));
        state.remove(Path::new("a.kdbx"));
        assert_eq!(state.current.as_deref(), Some(Path::new("b.kdbx")));
        state.remove(Path::new("b.kdbx"));
        assert_eq!(state.current, None);
    }

    #[test]
    fn a_database_is_titled_by_its_name_or_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pswm.json");
        let store = Store::load(path.clone());
        store.update(|s| {
            s.select(PathBuf::from(r"C:\Vault\base.kdbx"));
        })
        .unwrap();
        assert_eq!(store.read(|s| s.current().unwrap().title()), "base.kdbx");
        // No name kept: none written.
        assert!(!fs::read_to_string(&path).unwrap().contains("\"name\""));
        store.update(|s| s.current_mut().unwrap().name = Some("Home".into())).unwrap();
        assert_eq!(Store::load(path).read(|s| s.current().unwrap().title()), "Home");
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
