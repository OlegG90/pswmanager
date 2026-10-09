//! Changing entries (stage A2): the editor, a new entry, the recycle bin and
//! the star. The core applies each change to the working copy and saves it;
//! the change goes up about 10 s after the last one (`docs/spec-android.md`,
//! *Synchronisation*).

use crate::documents::Documents;
use pswm_core::edit::{self, EntryData, FileChange};
use pswm_core::generator;
use pswm_core::health;
use pswm_core::session::Session;
use pswm_core::similar;
use pswm_core::store::Store;
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

/// A change was saved: should the app go before it is sent, WorkManager
/// sends it later.
fn changed(app: &AppHandle) {
    crate::app::schedule_background_upload(app, &app.state::<Store>());
}

/// Syncs `UPLOAD_DELAY` after the last change. Going to the background first
/// sends it at once (`sync_if_pending`).
pub(crate) fn upload_soon(app: &AppHandle) {
    changed(app);
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
    } else {
        changed(&app);
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

/// Moves entries to the recycle bin.
#[tauri::command(async)]
pub fn delete_entries(app: AppHandle, session: State<Session>, ids: Vec<String>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.delete_entries(&ids)?;
        Ok(v.listing())
    })?;
    upload_soon(&app);
    Ok(listing)
}

/// The entries that are for the same site (Tools › Find similar entries).
#[tauri::command(async)]
pub fn similar_entries(session: State<Session>) -> Result<Vec<similar::Similar>, String> {
    session.read(|v| v.similar())
}

/// What merging entries into `keep` would change in it; protected values are left out.
#[tauri::command(async)]
pub fn merge_preview(session: State<Session>, keep: String, others: Vec<String>) -> Result<similar::Preview, String> {
    session.read(|v| v.merge_preview(&keep, &others))?
}

/// Merges entries into `keep`, moving them to the recycle bin.
#[tauri::command(async)]
pub fn merge_entries(app: AppHandle, session: State<Session>, keep: String, others: Vec<String>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.merge_entries(&keep, &others)?;
        Ok(v.listing())
    })?;
    upload_soon(&app);
    Ok(listing)
}

/// Puts an entry from the recycle bin back where it was.
#[tauri::command(async)]
pub fn restore_entry(app: AppHandle, session: State<Session>, id: String) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.restore(&[id])?;
        Ok(v.listing())
    })?;
    upload_soon(&app);
    Ok(listing)
}

/// Removes an entry in the recycle bin for good.
#[tauri::command(async)]
pub fn delete_for_good(app: AppHandle, session: State<Session>, id: String) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.delete_for_good(&[id])?;
        Ok(v.listing())
    })?;
    upload_soon(&app);
    Ok(listing)
}

/// Removes everything in the recycle bin for good.
#[tauri::command(async)]
pub fn empty_trash(app: AppHandle, session: State<Session>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.empty_bin()?;
        Ok(v.listing())
    })?;
    upload_soon(&app);
    Ok(listing)
}

/// Gives entries a tag or takes it off (the star is the tag Favorite); a tag
/// goes up with the next sync.
#[tauri::command(async)]
pub fn set_tag(app: AppHandle, session: State<Session>, ids: Vec<String>, tag: String, on: bool) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.set_tag(&ids, &tag, on)?;
        Ok(v.listing())
    })?;
    changed(&app);
    Ok(listing)
}

#[tauri::command(async)]
pub fn generate_password(options: generator::Options) -> Result<String, String> {
    generator::generate(&options).map(|password| password.to_string())
}

#[tauri::command(async)]
pub fn password_strength(password: String) -> health::Strength {
    health::strength(&Zeroizing::new(password))
}
