//! Files picked in the entry editor, held here until the entry is saved (or
//! the editor is cancelled, or the database locks): the window only gets
//! their names and sizes, never their content.

use crate::edit::{self, FileChange, FileEdit};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use zeroize::Zeroizing;

#[derive(Default)]
pub struct Staged(Mutex<Files>);

#[derive(Default)]
struct Files {
    next: u64,
    content: HashMap<u64, Zeroizing<Vec<u8>>>,
}

/// A staged file, as the editor shows it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedFile {
    /// What a [FileChange] names it by.
    pub content: u64,
    pub name: String,
    pub size: u64,
}

impl Staged {
    /// Holds a picked file until it is saved or released.
    pub fn add(&self, name: String, content: Zeroizing<Vec<u8>>) -> Result<StagedFile, String> {
        if content.len() > edit::MAX_ATTACHMENT {
            return Err(edit::too_big());
        }
        let mut files = self.0.lock().unwrap();
        files.next += 1;
        let id = files.next;
        let size = content.len() as u64;
        files.content.insert(id, content);
        Ok(StagedFile { content: id, name, size })
    }

    /// The changes with their content; the files stay staged until
    /// [Staged::release] (a save that fails can be tried again).
    pub fn resolve(&self, changes: &[FileChange]) -> Result<Vec<FileEdit>, String> {
        let files = self.0.lock().unwrap();
        let content = |id: &u64| files.content.get(id).cloned().ok_or("A file picked for this entry is gone; pick it again");
        changes
            .iter()
            .map(|change| {
                Ok(match change {
                    FileChange::Add { name, content: id } => FileChange::Add { name: name.clone(), content: content(id)? },
                    FileChange::Rename { name, to } => FileChange::Rename { name: name.clone(), to: to.clone() },
                    FileChange::Remove { name } => FileChange::Remove { name: name.clone() },
                })
            })
            .collect()
    }

    /// Lets go of these files (saved, or the editor dropped them).
    pub fn release(&self, ids: impl IntoIterator<Item = u64>) {
        let mut files = self.0.lock().unwrap();
        for id in ids {
            files.content.remove(&id);
        }
    }

    /// Lets go of every file (the database locked).
    pub fn clear(&self) {
        self.0.lock().unwrap().content.clear();
    }
}

impl<C> FileChange<C> {
    /// The staged content this change brings, if any.
    pub fn content(&self) -> Option<&C> {
        match self {
            FileChange::Add { content, .. } => Some(content),
            FileChange::Rename { .. } | FileChange::Remove { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_files_are_resolved_until_released() {
        let staged = Staged::default();
        let a = staged.add("a.txt".into(), Zeroizing::new(b"a".to_vec())).unwrap();
        assert_eq!((a.name.as_str(), a.size), ("a.txt", 1));
        let changes = [
            FileChange::Add { name: "a.txt".into(), content: a.content },
            FileChange::Rename { name: "b.txt".into(), to: "c.txt".into() },
            FileChange::Remove { name: "d.txt".into() },
        ];
        let edits = staged.resolve(&changes).unwrap();
        assert!(matches!(&edits[0], FileChange::Add { content, .. } if content.as_slice() == b"a"));
        assert_eq!(edits[2], FileChange::Remove { name: "d.txt".into() });
        staged.release(changes.iter().filter_map(FileChange::content).copied());
        assert!(staged.resolve(&changes).is_err());
        assert!(staged.add("big".into(), Zeroizing::new(vec![0; edit::MAX_ATTACHMENT + 1])).is_err());
    }
}
