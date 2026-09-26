//! Notices when the database file is changed by someone else (a sync client
//! delivering another device's change) while the app is open.

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// A sync client often writes a file in several steps; wait for it to settle.
const SETTLE: Duration = Duration::from_millis(700);

/// Calls `on_change` after changes to `file` settle, until the returned
/// watcher is dropped. The folder is watched, not the file: sync clients
/// replace a file by renaming a new one over it.
pub fn watch(file: &Path, on_change: impl Fn() + Send + 'static) -> notify::Result<RecommendedWatcher> {
    let name = file.file_name().map(|n| n.to_os_string());
    let folder = file.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let touches_file = event.is_ok_and(|e| e.paths.iter().any(|p| p.file_name().map(|n| n.to_os_string()) == name));
        if touches_file {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(&folder, RecursiveMode::NonRecursive)?;
    std::thread::spawn(move || {
        // Ends when the watcher (and with it the sender) is dropped.
        while rx.recv().is_ok() {
            while rx.recv_timeout(SETTLE).is_ok() {}
            on_change();
        }
    });
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn reports_changes_to_the_file_once_they_settle() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("base.kdbx");
        std::fs::write(&file, b"1").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let _watcher = watch(&file, move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        std::fs::write(dir.path().join("other.txt"), b"x").unwrap();
        std::thread::sleep(SETTLE * 2);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "another file");

        // Written the way sync clients do: a new file renamed over the old one, in quick steps.
        for step in 0..3 {
            std::fs::write(dir.path().join("incoming.tmp"), format!("v{step}")).unwrap();
            std::fs::rename(dir.path().join("incoming.tmp"), &file).unwrap();
        }
        std::thread::sleep(SETTLE * 3);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
