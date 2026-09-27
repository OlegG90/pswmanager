//! Attachments opened in another app. The app that opens a file needs it on
//! disk, so a copy is written to a folder of its own in the temp folder, read
//! only (changes made there are not saved back), and the folder is emptied
//! when the database locks, when the app quits and when it starts (after a
//! crash). A copy another app still holds open cannot be deleted on Windows;
//! the next clean-up tries again.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const FOLDER: &str = "pswm-open";

/// Where copies go: `%TEMP%\pswm-open`.
pub fn folder() -> PathBuf {
    std::env::temp_dir().join(FOLDER)
}

/// Writes `data` as `name` in a new subfolder of `folder` (so two files with
/// one name do not meet, and the other app shows the file's own name) and
/// returns its path.
pub fn write(folder: &Path, name: &str, data: &[u8]) -> io::Result<PathBuf> {
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    let dir = folder.join(crate::dbfile::hex(&random));
    fs::create_dir_all(&dir)?;
    let path = dir.join(file_name(name));
    fs::write(&path, data)?;
    let mut permissions = fs::metadata(&path)?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions)?;
    Ok(path)
}

/// Deletes every copy it can.
pub fn clean(folder: &Path) {
    let Ok(dirs) = fs::read_dir(folder) else { return };
    for dir in dirs.flatten() {
        if let Ok(files) = fs::read_dir(dir.path()) {
            for file in files.flatten() {
                // Windows does not delete a read-only file.
                if let Ok(metadata) = file.metadata() {
                    let mut permissions = metadata.permissions();
                    #[allow(clippy::permissions_set_readonly_false)] // the file is deleted right after
                    permissions.set_readonly(false);
                    let _ = fs::set_permissions(file.path(), permissions);
                }
                let _ = fs::remove_file(file.path());
            }
        }
        let _ = fs::remove_dir(dir.path());
    }
    let _ = fs::remove_dir(folder);
}

/// A name Windows accepts for a file: the last part of a path other clients
/// may store, without the characters Windows does not allow.
fn file_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let clean: String = last.chars().map(|c| if "<>:\"|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    match clean.trim().trim_end_matches('.') {
        "" => "file".into(),
        name => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_windows_accepts() {
        assert_eq!(file_name("scan.pdf"), "scan.pdf");
        assert_eq!(file_name("docs\\2024/scan.pdf"), "scan.pdf");
        assert_eq!(file_name("a:b?.txt"), "a_b_.txt");
        assert_eq!(file_name(" . "), "file");
        assert_eq!(file_name("notes."), "notes");
    }

    #[test]
    fn copies_are_read_only_and_cleaned_up() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join(FOLDER);
        let a = write(&folder, "scan.pdf", b"one").unwrap();
        let b = write(&folder, "scan.pdf", b"two").unwrap();
        assert_ne!(a, b);
        assert_eq!(a.file_name().unwrap(), "scan.pdf");
        assert_eq!(fs::read(&b).unwrap(), b"two");
        assert!(fs::metadata(&a).unwrap().permissions().readonly());
        clean(&folder);
        assert!(!folder.exists());
        clean(&folder); // nothing there: fine
    }
}
