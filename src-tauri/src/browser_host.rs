//! The app's side of browser integration (#273): the open database answers the
//! extension ([crate::browser::Host]), and what needs the user (connecting an
//! extension, replacing a saved login, choosing or making a passkey (#285),
//! unlocking) is asked in the window.

use crate::browser::{self, Host, Session};
use crate::native_messaging::{self, Clients};
use pswm_core::browser::{Group, Login};
use pswm_core::{generator, webauthn};
use pswm_core::session::Session as Vault;
use pswm_core::settings::Settings;
use pswm_core::store::Store;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// How long a question in the window waits for the user; the extension
/// gives up on its own after a while.
const ANSWER_WITHIN: Duration = Duration::from_secs(120);

/// The extensions connected, to tell them when the database locks or unlocks.
static CLIENTS: OnceLock<Clients> = OnceLock::new();
/// The pipe is served once, from the first time the integration is on.
static SERVING: AtomicBool = AtomicBool::new(false);

/// Questions waiting for the user's answer in the window, by number.
#[derive(Default)]
pub struct Questions {
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, mpsc::Sender<Value>>>,
}

/// Shows the window, asks it `event` with `details`, and waits for the
/// answer; `Value::Null` when there is none in time.
fn ask(app: &AppHandle, event: &str, mut details: Value) -> Value {
    let questions = app.state::<Questions>();
    let id = questions.next.fetch_add(1, Ordering::Relaxed);
    let (answer, answered) = mpsc::channel();
    questions.waiting.lock().unwrap().insert(id, answer);
    details["id"] = json!(id);
    crate::window::show(app);
    let _ = app.emit(event, details);
    let reply = answered.recv_timeout(ANSWER_WITHIN).unwrap_or(Value::Null);
    questions.waiting.lock().unwrap().remove(&id);
    reply
}

/// The window's answer to question `id`.
#[tauri::command]
pub fn browser_answer(questions: State<Questions>, id: u64, answer: Value) {
    if let Some(waiting) = questions.waiting.lock().unwrap().remove(&id) {
        let _ = waiting.send(answer);
    }
}

/// The extensions associated with the open database, for its settings.
#[tauri::command(async)]
pub fn browser_associations(session: State<Vault>) -> Result<Vec<pswm_core::browser::Association>, String> {
    session.read(|v| v.browser_associations())
}

/// Removes an association; the file is saved and synced like an edit.
#[tauri::command(async)]
pub fn forget_browser_association(app: AppHandle, name: String) -> Result<Vec<pswm_core::browser::Association>, String> {
    let session = app.state::<Vault>();
    let left = session.with_mut(|v| {
        v.browser_forget(&name)?;
        Ok(v.browser_associations())
    })?;
    crate::sync::upload_soon(&app);
    Ok(left)
}

struct AppHost(AppHandle);

impl AppHost {
    fn vault(&self) -> State<'_, Vault> {
        self.0.state::<Vault>()
    }

    /// A change to the database, saved and synced like an edit, the window
    /// told when it changed entries.
    fn change<R>(&self, change: impl FnOnce(&mut pswm_core::vault::Vault) -> Result<R, String>) -> Option<R> {
        let done = self.vault().with_mut(change).map_err(|e| eprintln!("The browser extension's change failed: {e}")).ok()?;
        crate::sync::upload_soon(&self.0);
        Some(done)
    }
}

impl Host for AppHost {
    fn unlocked(&self) -> bool {
        self.vault().is_unlocked()
    }

    fn ask_to_unlock(&self) {
        crate::window::show(&self.0);
    }

    fn hash(&self) -> Option<String> {
        self.vault().read(|v| v.browser_hash()).ok()
    }

    fn key(&self, name: &str) -> Option<String> {
        self.vault().read(|v| v.browser_key(name)).ok().flatten()
    }

    fn associate(&self, key: &str) -> Option<String> {
        let (database, taken) = self.vault().read(|v| (v.settings().name, v.browser_associations())).ok()?;
        let taken: Vec<String> = taken.into_iter().map(|a| a.name).collect();
        let name = ask(&self.0, "browser-associate", json!({ "database": database, "taken": taken }));
        let name = name.as_str()?.trim().to_string();
        self.change(|v| v.browser_associate(&name, key))?;
        Some(name)
    }

    fn logins(&self, url: &str) -> Vec<Login> {
        self.vault().read(|v| v.browser_logins(url)).unwrap_or_default()
    }

    fn totp(&self, uuid: &str) -> Option<String> {
        self.vault().read(|v| v.browser_totp(uuid)).ok().flatten()
    }

    fn groups(&self) -> Option<Group> {
        self.vault().read(|v| v.browser_groups()).ok()
    }

    fn create_group(&self, path: &str) -> Option<(String, String)> {
        self.change(|v| v.browser_create_group(path))
    }

    fn save_login(&self, uuid: Option<&str>, url: &str, login: &str, password: &str, group: Option<&str>) -> bool {
        // Replacing a login the database has is the user's call.
        if let Some(uuid) = uuid {
            let Ok(Some(entry)) = self.vault().read(|v| v.detail(uuid)) else { return false };
            let site = pswm_core::browser::Page::of_tab(url).map(|p| p.place()).unwrap_or_default();
            let details = json!({ "title": entry.summary.title, "username": entry.summary.username, "newUsername": login, "site": site });
            if ask(&self.0, "browser-update", details) != Value::Bool(true) {
                return false;
            }
        }
        let Some(id) = self.change(|v| v.browser_save_login(uuid, url, login, password, group)) else { return false };
        crate::show_changes(&self.0, vec![id]);
        true
    }

    fn generate_password(&self) -> Option<String> {
        let options = generator::Options { length: 20, upper: true, lower: true, digits: true, symbols: true, exclude_look_alikes: true };
        generator::generate(&options).ok().map(|p| p.to_string())
    }

    fn lock(&self) {
        crate::lock_now(&self.0);
    }

    fn passkeys_get(&self, mut public_key: Value, origin: &str, related: &[String]) -> Value {
        let (rp_id, origin) = match pswm_core::browser::check_passkey_request(&mut public_key, origin, related, false) {
            Ok(checked) => checked,
            Err(refusal) => return passkey_error(refusal as u8),
        };
        let request = public_key.to_string();
        let allowed = webauthn::sign_in_scope(&request).map(|scope| scope.allowed).unwrap_or_default();
        let Ok(offered) = self.vault().read(|v| v.passkeys_for(&rp_id, &allowed)) else { return passkey_error(DATABASE_NOT_OPENED) };
        // None here: the extension falls back to the browser's own passkeys.
        if offered.is_empty() {
            return passkey_error(NO_LOGINS_FOUND);
        }
        // Choosing in the window is the user's verification, as in KeePassXC.
        let chosen = ask(&self.0, "browser-passkey-get", json!({ "site": rp_id, "choices": offered }));
        let Some(id) = chosen.as_str().filter(|id| offered.iter().any(|c| c.id == *id)) else { return passkey_error(CANCELLED) };
        let caller = webauthn::Caller::Page { origin: origin.clone() };
        match self.vault().read(|v| v.sign_with_passkey(id, &request, &caller)) {
            Ok(Ok(credential)) => serde_json::from_str(&credential).unwrap_or_else(|_| passkey_error(UNKNOWN)),
            _ => passkey_error(UNKNOWN),
        }
    }

    fn passkeys_register(&self, mut public_key: Value, origin: &str, related: &[String], group: Option<&str>) -> Value {
        let (rp_id, origin) = match pswm_core::browser::check_passkey_request(&mut public_key, origin, related, true) {
            Ok(checked) => checked,
            Err(refusal) => return passkey_error(refusal as u8),
        };
        let request = public_key.to_string();
        let excluded = webauthn::excluded_ids(&request);
        let found = self.vault().read(|v| (!excluded.is_empty() && !v.passkeys_for(&rp_id, &excluded).is_empty(), v.entries_for_new_passkey(&rp_id)));
        let Ok((already, entries)) = found else { return passkey_error(DATABASE_NOT_OPENED) };
        if already {
            return passkey_error(CREDENTIAL_IS_EXCLUDED);
        }
        let user = public_key["user"]["name"].as_str().unwrap_or_default();
        let answer = ask(&self.0, "browser-passkey-register", json!({ "site": rp_id, "user": user, "entries": entries }));
        // Null: cancelled; `{ "entry": null }`: a new entry; `{ "entry": id }`: that entry.
        let Some(entry) = answer.get("entry") else { return passkey_error(CANCELLED) };
        let entry = entry.as_str().filter(|id| entries.iter().any(|c| c.id == *id));
        let Ok(made) = webauthn::make(&request, &webauthn::Caller::Page { origin: origin.clone() }) else { return passkey_error(UNKNOWN) };
        let Some(id) = self.change(|v| v.add_browser_passkey(entry, &made, group)) else { return passkey_error(UNKNOWN) };
        crate::show_changes(&self.0, vec![id]);
        serde_json::from_str(&made.response).unwrap_or_else(|_| passkey_error(UNKNOWN))
    }
}

/// KeePassXC's error codes a passkey answer may carry.
const DATABASE_NOT_OPENED: u8 = 1;
const NO_LOGINS_FOUND: u8 = 15;
const CREDENTIAL_IS_EXCLUDED: u8 = 21;
const CANCELLED: u8 = 22;
const UNKNOWN: u8 = 31;

/// A passkey answer that refuses, as KeePassXC writes it.
fn passkey_error(code: u8) -> Value {
    json!({ "errorCode": code })
}

/// Registers PswManager with the browsers and serves the extension, when the
/// setting is on; at start, and when it is turned on. Off, the registration
/// goes (another program's stays) and connections end.
pub fn apply(app: &AppHandle) -> Result<(), String> {
    let store = app.state::<Store>();
    if !Settings::of(&store).browser_integration() {
        native_messaging::unregister(store.dir());
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| format!("Cannot find PswManager's program: {e}"))?;
    native_messaging::register(store.dir(), &exe)?;
    if !SERVING.swap(true, Ordering::SeqCst) {
        let clients = CLIENTS.get_or_init(Clients::default);
        let (on, host) = (app.clone(), app.clone());
        let allowed = move || Settings::of(&on.state::<Store>()).browser_integration();
        let answer = move |session: &mut Session, request: &Value| session.handle(&AppHost(host.clone()), request);
        // A pipe another program holds: tried again the next time the setting is turned on.
        let failed = || SERVING.store(false, Ordering::SeqCst);
        native_messaging::serve(clients, failed, allowed, answer);
    }
    Ok(())
}

/// Tells the connected extensions that the database locked or unlocked.
pub fn tell(unlocked: bool) {
    if let Some(clients) = CLIENTS.get() {
        clients.tell(browser::signal(unlocked));
    }
}

/// Whether another program (KeePassXC) is registered for the extension.
pub fn other_registered(app: &AppHandle) -> bool {
    native_messaging::other_registered(app.state::<Store>().dir())
}
