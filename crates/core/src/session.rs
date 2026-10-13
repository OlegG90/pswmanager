//! The unlocked database, if any. Locking drops it, and with it every
//! decrypted value.

use crate::edit;
use crate::staged::Staged;
use crate::vault::Vault;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A clone is the same session: on Android the app and the credential
/// provider (which may run without the app's window) share one.
#[derive(Default, Clone)]
pub struct Session(Arc<Inner>);

#[derive(Default)]
struct Inner {
    unlocked: Mutex<Option<Unlocked>>,
    /// Files picked in the editor, not saved yet: they go when the database locks.
    staged: Staged,
}

/// What there is while the database is unlocked: each unlock starts afresh.
struct Unlocked {
    vault: Vault,
    key_needed: KeyNeeded,
    /// When the user was checked again for an export (#240).
    export_checked: Option<Instant>,
}

/// How long a check for an export allows it.
const EXPORT_WINDOW: Duration = Duration::from_secs(5 * 60);

/// Which copies of the unlocked database open with a key this device does not
/// know yet (another device changed it): the window offers to enter it.
#[derive(Clone, Copy, Default, PartialEq, Serialize)]
pub struct KeyNeeded {
    /// The file on this device, replaced by another program.
    pub local: bool,
    /// The remote file, at the last sync.
    pub remote: bool,
}

impl Session {
    pub fn with<R>(&self, f: impl FnOnce(&Vault) -> Option<R>) -> Result<R, String> {
        self.read(f)?.ok_or_else(|| edit::NOT_FOUND.into())
    }

    /// Like `with`, for reads that can only fail because the database is locked.
    pub fn read<R>(&self, f: impl FnOnce(&Vault) -> R) -> Result<R, String> {
        self.0.unlocked.lock().unwrap().as_ref().map(|u| f(&u.vault)).ok_or_else(|| "The database is locked".into())
    }

    pub fn with_mut<R>(&self, f: impl FnOnce(&mut Vault) -> Result<R, String>) -> Result<R, String> {
        let mut unlocked = self.0.unlocked.lock().unwrap();
        f(&mut unlocked.as_mut().ok_or("The database is locked")?.vault)
    }

    pub fn set(&self, vault: Option<Vault>) {
        *self.0.unlocked.lock().unwrap() = vault.map(|vault| Unlocked { vault, key_needed: KeyNeeded::default(), export_checked: None });
        self.0.staged.clear();
    }

    /// The files picked in the editor and not saved yet.
    pub fn staged(&self) -> &Staged {
        &self.0.staged
    }

    /// The user was checked again (the master password, Windows Hello, the
    /// fingerprint): an export of this database is allowed for a few minutes,
    /// until a file is written. Locking forgets it.
    pub fn allow_export(&self) -> Result<(), String> {
        let mut unlocked = self.0.unlocked.lock().unwrap();
        unlocked.as_mut().ok_or("The database is locked")?.export_checked = Some(Instant::now());
        Ok(())
    }

    /// The export [Session::allow_export] allows, while it does: its content
    /// and the file name offered. A file written with it calls [Session::exported].
    pub fn export(&self, format: crate::file_export::Format) -> Result<crate::file_export::Export, String> {
        let unlocked = self.0.unlocked.lock().unwrap();
        let unlocked = unlocked.as_ref().ok_or("The database is locked")?;
        if !unlocked.export_checked.is_some_and(|at| at.elapsed() < EXPORT_WINDOW) {
            return Err("Confirm it is you first: the master password, or Windows Hello or the fingerprint".into());
        }
        Ok(unlocked.vault.export_file(format))
    }

    /// A file was written: the next export is checked again.
    pub fn exported(&self) {
        if let Some(unlocked) = self.0.unlocked.lock().unwrap().as_mut() {
            unlocked.export_checked = None;
        }
    }

    pub fn is_unlocked(&self) -> bool {
        self.0.unlocked.lock().unwrap().is_some()
    }

    /// None while locked.
    pub fn key_needed(&self) -> KeyNeeded {
        self.0.unlocked.lock().unwrap().as_ref().map(|u| u.key_needed).unwrap_or_default()
    }

    /// Changes [KeyNeeded] while unlocked: what it was and is, or `None` when locked.
    pub fn change_key_needed(&self, change: impl FnOnce(&mut KeyNeeded)) -> Option<(KeyNeeded, KeyNeeded)> {
        let mut unlocked = self.0.unlocked.lock().unwrap();
        let needed = &mut unlocked.as_mut()?.key_needed;
        let before = *needed;
        change(needed);
        Some((before, *needed))
    }
}
