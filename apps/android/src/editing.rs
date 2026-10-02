//! Changing entries (stage A2): the editor, a new entry, the recycle bin and
//! the star. The core applies each change to the working copy and saves it;
//! the change goes up about 10 s after the last one (`docs/spec-android.md`,
//! *Synchronisation*).

use crate::documents::Documents;
use pswm_core::edit::{self, EntryData, FileChange};
use pswm_core::generator;
use pswm_core::health;
use pswm_core::session::Session;
use pswm_core::staged::StagedFile;
use pswm_core::vault::{Listing, Saved};
use std::time::Duration;
use tauri::{AppHandle, Manager, State, Wry};
use zeroize::Zeroizing;

/// How long after the last change it goes up.
const UPLOAD_DELAY: Duration = Duration::from_secs(10);

/// The upload waiting for the last change.
#[derive(Default)]
pub struct UploadSoon(crate::app::Latest);

/// Syncs `UPLOAD_DELAY` after the last change. Going to the background first
/// sends it at once (`sync_if_pending`).
fn upload_soon(app: &AppHandle) {
    let wait = app.state::<UploadSoon>().0.next();
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(UPLOAD_DELAY);
        if app.state::<UploadSoon>().0.is_latest(wait) {
            crate::app::start_sync(app);
        }
    });
}

/// The entry with every value, secrets included, for the editor.
#[tauri::command(async)]
pub fn edit_entry(session: State<Session>, id: String) -> Result<EntryData, String> {
    session.with(|v| v.edit_data(&id))
}

/// Creates (no `id`) or changes an entry with its `files`; `base` is the
/// entry as the editor opened it, so only what the editor changed is applied.
#[tauri::command(async)]
pub fn save_entry(
    app: AppHandle,
    session: State<Session>,
    id: Option<String>,
    base: Option<EntryData>,
    data: EntryData,
    files: Vec<FileChange>,
) -> Result<Saved, String> {
    let edits = session.staged().resolve(&files)?;
    let saved = session.with_mut(|v| {
        let (id, conflicts) = v.save_entry(id.as_deref(), base.as_ref(), &data, false, &edits)?;
        Ok(Saved { id, listing: v.listing(), conflicts })
    })?;
    session.staged().release_changes(&files);
    crate::icons::fetch(&app, &saved.listing);
    // Tags alone can wait for the next sync (going to the background, locking).
    if edit::needs_upload(base.as_ref(), &data, &files) {
        upload_soon(&app);
    }
    Ok(saved)
}

/// A file the user picks to attach in the editor, read and held here until
/// the entry is saved: the page gets its name and size only. Nothing when
/// cancelled.
#[tauri::command]
pub async fn pick_file_to_attach(app: AppHandle) -> Result<Option<StagedFile>, String> {
    crate::app::off_main(move || {
        let Some((name, content)) = app.state::<Documents<Wry>>().pick_to_read()? else { return Ok(None) };
        app.state::<Session>().staged().add(name, Zeroizing::new(content)).map(Some)
    })
    .await
}

/// The editor let go of files it had picked (cancelled, or removed them).
#[tauri::command]
pub fn release_files(session: State<Session>, files: Vec<u64>) {
    session.staged().release(files);
}

/// Moves the entry to the recycle bin.
#[tauri::command(async)]
pub fn delete_entry(app: AppHandle, session: State<Session>, id: String) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.delete_entries(&[id])?;
        Ok(v.listing())
    })?;
    upload_soon(&app);
    Ok(listing)
}

/// Stars the entry or takes the star off (the tag Favorite); like a tag, it
/// goes up with the next sync.
#[tauri::command(async)]
pub fn set_favorite(session: State<Session>, id: String, on: bool) -> Result<Listing, String> {
    session.with_mut(|v| {
        v.set_tag(&[id], edit::FAVORITE, on)?;
        Ok(v.listing())
    })
}

#[tauri::command(async)]
pub fn generate_password(options: generator::Options) -> Result<String, String> {
    generator::generate(&options).map(|password| password.to_string())
}

#[tauri::command(async)]
pub fn password_strength(password: String) -> health::Strength {
    health::strength(&Zeroizing::new(password))
}
