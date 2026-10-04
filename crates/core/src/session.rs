//! The unlocked database, if any. Locking drops it, and with it every
//! decrypted value.

use crate::edit;
use crate::staged::Staged;
use crate::vault::Vault;
use serde::Serialize;
use std::sync::{Arc, Mutex};

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
}

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
        *self.0.unlocked.lock().unwrap() = vault.map(|vault| Unlocked { vault, key_needed: KeyNeeded::default() });
        self.0.staged.clear();
    }

    /// The files picked in the editor and not saved yet.
    pub fn staged(&self) -> &Staged {
        &self.0.staged
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
