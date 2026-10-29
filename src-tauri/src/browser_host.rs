//! The app's side of browser integration (#273): the open database answers the
//! extension ([crate::browser::Host]), and what needs the user (connecting an
//! extension, replacing a saved login, choosing or making a passkey (#285),
//! unlocking) is asked in the window.

use crate::browser::{self, Failure, Host, Session};
use crate::native_messaging::{self, Clients};
use crate::questions::{Questions, SystemClock, Transport};
use pswm_core::browser::{Group, Login};
use pswm_core::{generator, webauthn};
use pswm_core::session::Session as Vault;
use pswm_core::settings::Settings;
use pswm_core::store::Store;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager, State};

/// The extensions connected, to tell them when the database locks or unlocks.
static CLIENTS: OnceLock<Clients> = OnceLock::new();
/// The pipe is served once, from the first time the integration is on.
static SERVING: AtomicBool = AtomicBool::new(false);

/// The app's side of a question: shows it in the window, and is told when
/// it expired unanswered.
struct AppTransport {
    app: AppHandle,
}

impl Transport for AppTransport {
    fn send(&self, event: &str, details: Value) {
        crate::window::show(&self.app);
        let _ = self.app.emit(event, details);
    }

    fn expired(&self, id: u64) {
        let _ = self.app.emit("browser-expired", id);
    }
}

/// Shows the window, asks it `event` with `details`, and waits for the
/// answer; `Value::Null` when there is none in time.
fn ask(app: &AppHandle, event: &str, details: Value) -> Value {
    let transport = AppTransport { app: app.clone() };
    app.state::<Questions>().ask(&transport, &SystemClock, event, details)
}

/// Drops the questions still waiting (the database locked): each is answered
/// as cancelled, so an answer given after cannot reach the database opened
/// next.
pub fn forget_questions(app: &AppHandle) {
    app.state::<Questions>().forget();
}

/// The window's answer to question `id`.
#[tauri::command]
pub fn browser_answer(questions: State<Questions>, id: u64, answer: Value) {
    questions.answer(id, answer);
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
        let asked_about = self.hash()?;
        let (database, taken) = self.vault().read(|v| (v.settings().name, v.browser_associations())).ok()?;
        let taken: Vec<String> = taken.into_iter().map(|a| a.name).collect();
        let name = ask(&self.0, "browser-associate", json!({ "database": database, "taken": taken }));
        let name = name.as_str()?.trim().to_string();
        // The answer is for the database asked about, not one opened since.
        if self.hash().as_deref() != Some(asked_about.as_str()) {
            return None;
        }
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
        // Replacing a login the database has is the user's call, asked only
        // about an entry the extension may change.
        if let Some(uuid) = uuid {
            let Ok(Ok((title, username))) = self.vault().read(|v| v.browser_replaced(uuid, url)) else { return false };
            let site = pswm_core::browser::Page::of_tab(url).map(|p| p.place()).unwrap_or_default();
            let details = json!({ "title": title, "username": username, "newUsername": login, "site": site });
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
        let Ok(offered) = self.vault().read(|v| v.browser_passkeys(&rp_id, &allowed)) else { return failed(Failure::DatabaseNotOpened) };
        // None here: the extension falls back to the browser's own passkeys.
        if offered.is_empty() {
            return failed(Failure::NoLogins);
        }
        // Choosing in the window is the user's verification, as in KeePassXC.
        let chosen = ask(&self.0, "browser-passkey-get", json!({ "site": rp_id, "choices": offered }));
        let Some(id) = chosen.as_str().filter(|id| offered.iter().any(|c| c.id == *id)) else { return failed(Failure::PasskeyCancelled) };
        let caller = webauthn::Caller::Page { origin };
        credential(self.vault().read(|v| v.sign_with_passkey(id, &request, &caller)).ok().and_then(Result::ok))
    }

    fn passkeys_register(&self, mut public_key: Value, origin: &str, related: &[String], group: Option<&str>) -> Value {
        let (rp_id, origin) = match pswm_core::browser::check_passkey_request(&mut public_key, origin, related, true) {
            Ok(checked) => checked,
            Err(refusal) => return passkey_error(refusal as u8),
        };
        let request = public_key.to_string();
        let excluded = webauthn::excluded_ids(&request);
        let found = self.vault().read(|v| (!excluded.is_empty() && !v.browser_passkeys(&rp_id, &excluded).is_empty(), v.browser_entries_for_new_passkey(&rp_id)));
        let Ok((already, entries)) = found else { return failed(Failure::DatabaseNotOpened) };
        if already {
            return failed(Failure::CredentialIsExcluded);
        }
        let user = public_key["user"]["name"].as_str().unwrap_or_default();
        let answer = ask(&self.0, "browser-passkey-register", json!({ "site": rp_id, "user": user, "entries": entries }));
        // Null: cancelled; `{ "entry": null }`: a new entry; `{ "entry": id }`:
        // that entry, which must be one offered.
        let entry = match answer.get("entry") {
            Some(Value::Null) => None,
            Some(Value::String(id)) if entries.iter().any(|c| c.id == *id) => Some(id.as_str()),
            _ => return failed(Failure::PasskeyCancelled),
        };
        let Ok(made) = webauthn::make(&request, &webauthn::Caller::Page { origin }) else { return failed(Failure::PasskeyFailed) };
        let Some(id) = self.change(|v| v.add_browser_passkey(entry, &made, group)) else { return failed(Failure::PasskeyFailed) };
        crate::show_changes(&self.0, vec![id]);
        credential(Some(made.response))
    }
}

/// A passkey answer that refuses, as KeePassXC writes it.
fn passkey_error(code: u8) -> Value {
    json!({ "errorCode": code })
}

fn failed(failure: Failure) -> Value {
    passkey_error(failure as u8)
}

/// The credential the core made or signed, as the answer carries it.
fn credential(json: Option<String>) -> Value {
    json.and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_else(|| failed(Failure::PasskeyFailed))
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
        // A pipe another program (or another user, who may know the name)
        // holds: tried again the next time the setting is turned on.
        let told = app.clone();
        let failed = move || {
            SERVING.store(false, Ordering::SeqCst);
            let _ = told.emit("notice", UNSERVED);
        };
        native_messaging::serve(clients, failed, allowed, answer);
    }
    Ok(())
}

/// What the user is told when the extension cannot be served.
const UNSERVED: &str = "The browser extension cannot connect: another program holds its connection. Turn the integration off and on again in the settings";

/// Whether the integration is on but the extension cannot be served (see [apply]).
pub fn unserved(app: &AppHandle) -> bool {
    Settings::of(&app.state::<Store>()).browser_integration() && !SERVING.load(Ordering::SeqCst)
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
