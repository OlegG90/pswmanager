//! The open database's entries: reading, editing and copying them, their
//! history, attachments, tags and trash, and the tools over them (merging,
//! similar entries, password health, the generator).
use crate::{clipboard, pick, sync};
use crate::databases::fetch_icons;
use pswm_core::{edit, generator, health, icons, opened, otp, qr, similar, staged, vault};
use pswm_core::edit::EntryData;
use pswm_core::session::Session;
use pswm_core::settings::Settings;
use pswm_core::store::Store;
use pswm_core::vault::{EntryDetail, Listing, Saved, Vault, Version, VersionDetail};
use tauri::{AppHandle, State, Window};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use zeroize::Zeroizing;

#[tauri::command(async)]
pub fn listing(session: State<Session>) -> Result<Listing, String> {
    session.read(Vault::listing)
}

#[tauri::command(async)]
pub fn entry(session: State<Session>, id: String) -> Result<EntryDetail, String> {
    session.with(|v| v.detail(&id))
}

/// The entry's older versions: when and what changed, never values.
#[tauri::command(async)]
pub fn entry_history(session: State<Session>, id: String) -> Result<Vec<Version>, String> {
    session.with(|v| v.history(&id))
}

/// One older version, read only, secrets masked.
#[tauri::command(async)]
pub fn entry_version(session: State<Session>, id: String, index: usize) -> Result<VersionDetail, String> {
    session.with(|v| v.version(&id, index))
}

/// Makes an older version the entry's current content; saves and syncs as an edit.
#[tauri::command(async)]
pub fn restore_version(app: AppHandle, session: State<Session>, id: String, index: usize, saved: Option<String>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.restore_version(&id, index, saved.as_deref())?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(listing)
}

/// One field's value (of an older version with `version`), for showing it.
/// Asked for only on "reveal".
#[tauri::command(async)]
pub fn reveal(session: State<Session>, id: String, field: String, version: Option<usize>) -> Result<String, String> {
    session.with(|v| v.field_in(&id, version, &field)).map(|value| value.to_string())
}

/// Copies a field without the value passing through the frontend. Returns
/// the seconds until the clipboard is cleared.
#[tauri::command(async)]
pub fn copy_field(store: State<Store>, session: State<Session>, id: String, field: String, version: Option<usize>) -> Result<u64, String> {
    let value = session.with(|v| v.field_in(&id, version, &field))?;
    if value.is_empty() {
        return Err(format!("{field} is empty"));
    }
    let clear_after = Settings::of(&store).clear_clipboard_after();
    clipboard::copy(value, clear_after)?;
    Ok(clear_after.as_secs())
}

/// Opens the entry's URL, or the further URL `field` (`KP2A_URL_n`), in the
/// default browser (web addresses only).
#[tauri::command(async)]
pub fn open_url(app: AppHandle, session: State<Session>, id: String, field: Option<String>) -> Result<(), String> {
    let url = session.with(|v| Some(v.link(&id, field.as_deref())))?.ok_or("Not a web address")?;
    app.opener().open_url(url.as_str(), None::<&str>).map_err(|e| e.to_string())
}

/// The entry with every value, secrets included, for the editor.
#[tauri::command(async)]
pub fn edit_entry(session: State<Session>, id: String) -> Result<EntryData, String> {
    session.with(|v| v.edit_data(&id))
}

/// Creates (no `id`) or changes an entry with its `files`, saves the file
/// and returns the new listing with the entry's id.
#[tauri::command(async)]
pub fn save_entry(
    app: AppHandle,
    session: State<Session>,
    id: Option<String>,
    base: Option<EntryData>,
    data: EntryData,
    template: Option<bool>,
    files: Vec<edit::FileChange>,
) -> Result<Saved, String> {
    let edits = session.staged().resolve(&files)?;
    let saved = session.with_mut(|v| {
        let (id, conflicts) = v.save_entry(id.as_deref(), base.as_ref(), &data, template.unwrap_or(false), &edits)?;
        Ok(Saved { id, listing: v.listing(), conflicts })
    })?;
    session.staged().release_changes(&files);
    fetch_icons(&app, &saved.listing);
    // Tags alone can wait for the next sync (hiding, locking, quitting).
    if edit::needs_upload(base.as_ref(), &data, &files) {
        sync::upload_soon(&app);
    }
    Ok(saved)
}

/// Saves one of the entry's files where the user chooses. The content goes
/// from the database to the file without passing through the frontend.
/// False when the user cancelled.
#[tauri::command(async)]
pub fn save_attachment(window: Window, session: State<Session>, id: String, name: String, version: Option<usize>) -> Result<bool, String> {
    let data = session.with(|v| v.attachment(&id, version, &name))?;
    // A name is a file name, but other clients may store a path.
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(&name);
    let chosen = window.dialog().file().set_parent(&window).set_file_name(file_name).blocking_save_file();
    let Some(path) = chosen.map(|p| p.into_path().map_err(|e| e.to_string())).transpose()? else { return Ok(false) };
    std::fs::write(&path, &*data).map_err(|e| format!("Cannot save the file: {e}"))?;
    Ok(true)
}

/// Opens one of the entry's files in the app Windows uses for its type,
/// from a read-only copy that is deleted when the database locks.
#[tauri::command(async)]
pub fn open_attachment(app: AppHandle, session: State<Session>, id: String, name: String, version: Option<usize>) -> Result<(), String> {
    if opened::is_runnable(&name) {
        return Err("Programs and scripts are not opened from the database; save the file to run it".into());
    }
    let data = session.with(|v| v.attachment(&id, version, &name))?;
    let path = opened::write(&opened::folder(), &name, &data).map_err(|e| format!("Cannot open the file: {e}"))?;
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| format!("Cannot open the file: {e}"))
}

/// A file the user picks to attach in the editor, read and held in the
/// backend until the entry is saved: the window gets its name and size only.
/// Nothing when cancelled.
#[tauri::command(async)]
pub fn pick_file_to_attach(window: Window, session: State<Session>) -> Result<Option<staged::StagedFile>, String> {
    let Some(path) = pick(&window, "", &[])? else { return Ok(None) };
    let content = read_picked(&path, edit::check_size)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    session.staged().add(name, content).map(Some)
}

/// A TOTP secret from a QR code (#279) for the editor: the picture on the
/// clipboard (a `Win+Shift+S` snip of the site's code), or, when it holds no
/// such code, a picture the user picks. Nothing when they cancel.
#[tauri::command(async)]
pub fn scan_totp_qr(window: Window) -> Result<Option<String>, String> {
    Ok(read_qr(&window, qr::from_rgba, qr::from_image, |_| true)?.map(|uri| uri.to_string()))
}

/// A QR code's text for `from_clipboard` / `from_file` to take: from the
/// picture on the clipboard (when `new` says it is new), or else a picture
/// the user picks. Nothing when they cancel.
pub(crate) fn read_qr(
    window: &Window,
    from_clipboard: qr::FromPixels,
    from_file: qr::FromFile,
    new: impl Fn(&str) -> bool,
) -> Result<Option<Zeroizing<String>>, String> {
    if let Some(text) = clipboard::qr(from_clipboard).filter(|text| new(text)) {
        return Ok(Some(text));
    }
    let Some(path) = pick(window, "Pictures", &["png", "jpg", "jpeg"])? else { return Ok(None) };
    let content = read_picked(&path, qr::check_size)?;
    from_file(&content).map(Some)
}

/// A file the user picked, read whole; `check_size` refuses one too big
/// before it is read.
fn read_picked(path: &std::path::Path, check_size: fn(u64) -> Result<(), String>) -> Result<Zeroizing<Vec<u8>>, String> {
    let unreadable = |e: std::io::Error| format!("Cannot read the file: {e}");
    check_size(std::fs::metadata(path).map_err(unreadable)?.len())?;
    std::fs::read(path).map(Zeroizing::new).map_err(unreadable)
}

/// The editor let go of files it had picked (cancelled, or removed them).
#[tauri::command(async)]
pub fn release_files(session: State<Session>, files: Vec<u64>) {
    session.staged().release(files);
}

/// Gives entries a tag or takes it off (the star is the tag Favorite),
/// saves the file and returns the new listing. Tags wait for the next sync.
#[tauri::command(async)]
pub fn set_tag(session: State<Session>, ids: Vec<String>, tag: String, on: bool) -> Result<Listing, String> {
    session.with_mut(|v| {
        v.set_tag(&ids, &tag, on)?;
        Ok(v.listing())
    })
}

/// Renames a tag in every entry; tags wait for the next sync.
#[tauri::command(async)]
pub fn rename_tag(session: State<Session>, from: String, to: String) -> Result<Listing, String> {
    session.with_mut(|v| {
        v.rename_tag(&from, &to)?;
        Ok(v.listing())
    })
}

/// Takes a tag off every entry; tags wait for the next sync.
#[tauri::command(async)]
pub fn remove_tag(session: State<Session>, tag: String) -> Result<Listing, String> {
    session.with_mut(|v| {
        v.remove_tag(&tag)?;
        Ok(v.listing())
    })
}

/// Puts entries from the recycle bin back where they were, saves the file and
/// returns the new listing.
#[tauri::command(async)]
pub fn restore_entries(app: AppHandle, session: State<Session>, ids: Vec<String>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.restore(&ids)?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(listing)
}

/// Removes entries in the recycle bin for good, saves the file and returns
/// the new listing.
#[tauri::command(async)]
pub fn delete_for_good(app: AppHandle, session: State<Session>, ids: Vec<String>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.delete_for_good(&ids)?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(listing)
}

/// Removes everything in the recycle bin for good, saves the file and returns
/// the new listing.
#[tauri::command(async)]
pub fn empty_trash(app: AppHandle, session: State<Session>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.empty_bin()?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(listing)
}

/// Copies an entry next to it (#255), saves the file and returns the copy
/// with the new listing.
#[tauri::command(async)]
pub fn duplicate_entry(app: AppHandle, session: State<Session>, id: String) -> Result<Saved, String> {
    let saved = session.with_mut(|v| {
        let id = v.duplicate(&id)?;
        Ok(Saved { id, listing: v.listing(), conflicts: Vec::new() })
    })?;
    sync::upload_soon(&app);
    Ok(saved)
}

/// Moves entries to the recycle bin, saves the file and returns the new listing.
#[tauri::command(async)]
pub fn delete_entries(app: AppHandle, session: State<Session>, ids: Vec<String>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.delete_entries(&ids)?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(listing)
}

/// Merges entries into `keep`, moving them to the recycle bin; saves the file
/// and returns the new listing.
#[tauri::command(async)]
pub fn merge_entries(app: AppHandle, session: State<Session>, keep: String, others: Vec<String>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.merge_entries(&keep, &others)?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(listing)
}

/// What merging entries into `keep` would change in it; protected values are left out.
#[tauri::command(async)]
pub fn merge_preview(session: State<Session>, keep: String, others: Vec<String>) -> Result<vault::MergePreview, String> {
    session.read(|v| v.merge_preview(&keep, &others))?
}

/// An image file for an entry's own icon, as base64; `None` when cancelled.
#[tauri::command(async)]
pub fn pick_icon_image(window: Window) -> Result<Option<String>, String> {
    let Some(path) = pick(&window, "Image", &["png", "jpg", "jpeg", "gif", "webp"])? else { return Ok(None) };
    let bytes = std::fs::read(&path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    edit::check_icon_image(&bytes)?;
    Ok(Some(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)))
}

/// The entry's current TOTP code, or nothing when it has no secret.
#[tauri::command(async)]
pub fn totp(session: State<Session>, id: String) -> Result<Option<otp::Code>, String> {
    session.read(|v| v.totp(&id))?
}

/// Copies the current TOTP code; returns the seconds until the clipboard is cleared.
#[tauri::command(async)]
pub fn copy_totp(store: State<Store>, session: State<Session>, id: String) -> Result<u64, String> {
    let code = session.read(|v| v.totp(&id))??.ok_or("The entry has no TOTP secret")?;
    let clear_after = Settings::of(&store).clear_clipboard_after();
    clipboard::copy(Zeroizing::new(code.code), clear_after)?;
    Ok(clear_after.as_secs())
}

#[tauri::command(async)]
pub fn generate_password(options: generator::Options) -> Result<String, String> {
    generator::generate(&options).map(|password| password.to_string())
}

#[tauri::command(async)]
pub fn password_strength(password: String) -> health::Strength {
    health::strength(&Zeroizing::new(password))
}

/// Reused, weak and old passwords; worked out here, so no password leaves the backend.
#[tauri::command(async)]
pub fn password_health(session: State<Session>) -> Result<health::Health, String> {
    session.read(Vault::health)
}

/// The entries alike in `criteria` (the same site and account when not
/// given), by their ids.
#[tauri::command(async)]
pub fn similar_entries(session: State<Session>, criteria: Option<similar::Criteria>) -> Result<Vec<similar::Similar>, String> {
    session.read(|v| v.similar(criteria.unwrap_or_default()))
}

/// A cached site icon as a `data:` URL, or nothing yet.
#[tauri::command(async)]
pub fn icon(cache: State<icons::Cache>, host: String) -> Option<String> {
    cache.get(&host).and_then(|bytes| icons::data_url(&bytes))
}
