//! Auto-Type from the hotkey to the keystrokes: the window it types into, the
//! chooser when more than one entry fits, and typing one at a time. The typing
//! itself is `autotype`.
use crate::{autotype, chooser, window, AutoTypeNotice, AutoTypeTarget};
use pswm_core::autotype::Typed;
use pswm_core::session::Session;
use pswm_core::settings::Settings;
use pswm_core::vault;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Auto-Type's choice, when the window asks which entry to type.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AutoTypeAsk {
    /// The title of the window typed into.
    window: String,
    /// A browser's site, when it could be read.
    site: Option<String>,
    browser: bool,
    /// The entries that fit it.
    matches: Vec<vault::EntryChoice>,
}

/// Auto-Type's hotkey (#272): the window in front waits to be typed into. One
/// entry alone fits it: typed at once (unless the user wants to be asked).
/// Otherwise, or while locked, the chooser comes up over it to unlock or
/// choose (#307); the main window stays as it is.
pub(crate) fn auto_type_pressed(app: &AppHandle) {
    // A press while one is typing (the hotkey held, pressed twice) types nothing more.
    let Some(_typing) = Typing::start() else { return };
    let Some(target) = autotype::foreground() else {
        // Pressed in the main window: said so, rather than nothing happening
        // (in the chooser, which asks already, nothing needs saying).
        if window::is_focused(app) {
            let _ = app.emit("notice", "Auto-Type types into another window: press the hotkey there, or use the entry's Auto-Type (Ctrl+Shift+V)");
        }
        return; // the desktop, the taskbar
    };
    let over = target.bounds();
    *app.state::<AutoTypeTarget>().0.lock().unwrap() = Some(target);
    *app.state::<AutoTypeNotice>().0.lock().unwrap() = None;
    if app.state::<Session>().is_unlocked() {
        match waiting_window(app, true) {
            // Typed: a chooser still up from an earlier press has nothing to ask now.
            Ok(None) => return chooser::hide(app),
            Ok(Some(_)) => {}
            Err(message) => *app.state::<AutoTypeNotice>().0.lock().unwrap() = Some(message),
        }
    }
    // Once up (and unlocked), the chooser asks for what waits (auto_type_waiting).
    chooser::ask(app, over);
}

/// What the chooser shows once it is up and unlocked: why the hotkey's window
/// was not typed into, or the choice for it; nothing when none waits. Asked
/// after every unlock: the user may have unlocked for something else.
#[tauri::command(async)]
pub fn auto_type_waiting(app: AppHandle) -> Result<Option<AutoTypeAsk>, String> {
    if let Some(message) = app.state::<AutoTypeNotice>().0.lock().unwrap().take() {
        return Err(message);
    }
    waiting_window(&app, false)
}

/// What the window waiting to be typed into needs: nothing when there is none,
/// or when `may_type` and one entry alone fits it (typed now, unless the user
/// wants to be asked); else the choice to show, and the window waits on. Taken
/// while deciding, so two calls never both type.
fn waiting_window(app: &AppHandle, may_type: bool) -> Result<Option<AutoTypeAsk>, String> {
    let Some(target) = take_waiting(app) else { return Ok(None) };
    let matches = app.state::<Session>().read(|v| v.auto_type_matches(&target.window))?;
    let always_ask = Settings::of(&app.state()).auto_type_always_ask();
    if let chooser::Plan::Type { id } = chooser::plan(may_type, always_ask, &matches) {
        return type_entry(app, &target, &id, Typed::Sequence).map(|()| None);
    }
    let site = target.window.page.as_ref().map(pswm_core::browser::Page::place);
    let ask = AutoTypeAsk { window: target.window.title.clone(), site, browser: target.window.browser, matches };
    // A newer press meanwhile is the one that waits. Locked meanwhile, none
    // does: checked under the lock lock_now takes the window with.
    let waiting_target = app.state::<AutoTypeTarget>();
    let mut waiting = waiting_target.0.lock().unwrap();
    if !app.state::<Session>().is_unlocked() {
        return Ok(None);
    }
    waiting.get_or_insert(target);
    Ok(Some(ask))
}

/// Auto-Type typing now: one at a time, so keys of two are never mixed.
struct Typing;

static TYPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl Typing {
    /// The turn to type; `None` while another typing runs.
    fn start() -> Option<Typing> {
        (!TYPING.swap(true, std::sync::atomic::Ordering::SeqCst)).then_some(Typing)
    }
}

impl Drop for Typing {
    fn drop(&mut self) {
        TYPING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Types the entry the user chose in the chooser into the window waiting for
/// it. The chooser steps aside first and comes back when typing fails, to say why.
#[tauri::command(async)]
pub fn auto_type(app: AppHandle, id: String, typed: Typed) -> Result<(), String> {
    let _typing = Typing::start().ok_or("Auto-Type is typing already")?;
    let target = take_waiting(&app).ok_or("The window to type into is gone: press the Auto-Type hotkey there again")?;
    chooser::hide(&app);
    let done = type_entry(&app, &target, &id, typed);
    match done {
        Ok(()) => chooser_gone(&app),
        Err(_) => chooser::show(&app),
    }
    done
}

/// The user chose no entry: nothing waits to be typed into, and the chooser goes.
#[tauri::command(async)]
pub fn cancel_auto_type(app: AppHandle) {
    take_waiting(&app);
    app.state::<AutoTypeNotice>().0.lock().unwrap().take();
    chooser::hide(&app);
    chooser_gone(&app);
}

/// The chooser has gone. With the main window hidden, PswManager is put away:
/// it locks or syncs as hiding the main window does.
fn chooser_gone(app: &AppHandle) {
    if !window::is_shown(app) {
        window::settle(app);
    }
}

/// The window waiting to be typed into, if it still is, taken: no other call gets it.
fn take_waiting(app: &AppHandle) -> Option<autotype::Target> {
    app.state::<AutoTypeTarget>().0.lock().unwrap().take().filter(autotype::Target::is_current)
}

/// Entry-level Auto-Type: the window steps aside, and the entry is typed into
/// the window that comes forward then (the one in front before PswManager).
#[tauri::command(async)]
pub fn auto_type_entry(app: AppHandle, id: String, typed: Typed) -> Result<(), String> {
    let _typing = Typing::start().ok_or("Auto-Type is typing already")?;
    // An entry that cannot be typed (Auto-Type off, no TOTP secret) says so
    // before the window steps aside.
    app.state::<Session>().read(|v| v.auto_type_steps(&id, "", typed))??;
    if !window::step_aside(&app) {
        return Err("Auto-Type types from the window".into());
    }
    let done = autotype::behind().ok_or_else(|| "There is no window behind PswManager to type into".to_string()).and_then(|target| type_entry(&app, &target, &id, typed));
    window::come_back(&app, done.is_ok());
    done
}

/// Types `typed` of entry `id` into `target`.
fn type_entry(app: &AppHandle, target: &autotype::Target, id: &str, typed: Typed) -> Result<(), String> {
    let steps = app.state::<Session>().read(|v| v.auto_type_steps(id, &target.window.title, typed))??;
    // What the window shows now may refuse the typing: an unreadable
    // browser's address, or a page gone on since.
    if let Some(refusal) = pswm_core::autotype::refusal(&target.window, autotype::page_now(target).as_ref()) {
        return Err(refusal);
    }
    if autotype::runs_higher(target) {
        return Err(format!("\"{}\" runs as administrator: Auto-Type cannot type into it unless PswManager does too", target.window.title));
    }
    autotype::type_into(target, &steps, || app.state::<Session>().is_unlocked())
}
