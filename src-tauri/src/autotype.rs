//! Auto-Type on Windows (#272): the window the hotkey was pressed in, the
//! address a browser there shows (read through UI Automation), and the core's
//! steps typed into it with `SendInput`.

use pswm_core::autotype::{site_of_address, Key, Mods, Step, Window};
use std::time::{Duration, Instant};
use windows::core::BSTR;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationValuePattern, TreeScope_Descendants, UIA_AutomationIdPropertyId, UIA_DocumentControlTypeId,
    UIA_ControlTypePropertyId, UIA_EditControlTypeId, UIA_ValuePatternId,
};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows_sys::Win32::Security::{GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel, TOKEN_MANDATORY_LABEL, TOKEN_QUERY};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardLayout, SendInput, VkKeyScanExW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1,
    VK_HOME, VK_INSERT, VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
};

/// Browsers by their program's file name: one of them is a browser even when
/// its address cannot be read (and then nothing fits it).
const BROWSERS: [&str; 6] = ["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "vivaldi.exe", "opera.exe"];
/// Windows that are no app to type into: the desktop and the taskbar.
const NOT_APPS: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];
/// How long a window waits to be typed into while the user unlocks or chooses.
const KEPT_FOR: Duration = Duration::from_secs(120);
/// The pause between keys unless a sequence sets another (`{DELAY=…}`).
const KEY_DELAY: u32 = 10;

/// A window to type into, as it was when the hotkey was pressed.
#[derive(Debug, Clone)]
pub struct Target {
    /// The window's handle, as a number: a handle is not `Send`.
    hwnd: usize,
    /// A browser's engine, to read its address again.
    engine: Option<Engine>,
    pub window: Window,
    since: Instant,
}

impl Target {
    fn hwnd(&self) -> HWND {
        self.hwnd as HWND
    }

    /// Still worth typing into: the window is there, and it was chosen not long ago.
    pub fn is_current(&self) -> bool {
        // SAFETY: IsWindow takes any value.
        self.since.elapsed() < KEPT_FOR && unsafe { IsWindow(self.hwnd()) } != 0
    }
}

/// The window in front: not one of PswManager's own, nor the desktop or the
/// taskbar. A browser's site is read from its address bar.
pub fn foreground() -> Option<Target> {
    // SAFETY: plain Win32 calls; every buffer is as long as said.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let mut class = [0u16; 64];
        let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        let class = String::from_utf16_lossy(&class[..len]);
        if pid == GetCurrentProcessId() || NOT_APPS.contains(&class.as_str()) {
            return None;
        }
        let mut title = [0u16; 512];
        let len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
        let title = String::from_utf16_lossy(&title[..len]);
        let engine = Engine::of(&class);
        let address = engine.and_then(|e| address(hwnd, e));
        // A browser not in the list counts as one when it shows an address:
        // its title, which the page chooses, must never pick the entry.
        let browser = address.is_some() || program(pid).is_some_and(|p| BROWSERS.contains(&p.as_str()));
        let site = address.as_deref().and_then(site_of_address);
        Some(Target { hwnd: hwnd as usize, engine, window: Window { title, browser, site }, since: Instant::now() })
    }
}

/// The site the target's browser shows now; `None` when it is no browser or
/// its address cannot be read.
pub fn site_now(target: &Target) -> Option<String> {
    target.engine.and_then(|e| address(target.hwnd(), e)).as_deref().and_then(site_of_address)
}

/// The browser engines whose address bar can be read, by their windows' class.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Engine {
    /// Firefox and the browsers made from it.
    Gecko,
    /// Chrome, Edge and the browsers made from Chromium (and apps made with
    /// Electron, which show no address).
    Chromium,
}

impl Engine {
    fn of(class: &str) -> Option<Engine> {
        match class {
            "MozillaWindowClass" => Some(Engine::Gecko),
            "Chrome_WidgetWin_1" => Some(Engine::Chromium),
            _ => None,
        }
    }
}

/// The file name of process `pid`'s program, in lower case.
fn program(pid: u32) -> Option<String> {
    // SAFETY: the handle is closed below; the buffer is as long as said.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut path = [0u16; 1024];
        let mut len = path.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut len) != 0;
        CloseHandle(process);
        let path = String::from_utf16_lossy(&path[..len as usize]);
        ok.then(|| path.rsplit('\\').next().unwrap_or_default().to_lowercase())
    }
}

/// What a browser window's address bar holds: Firefox's by its id, the
/// Chromium browsers' as the window's first text field (the toolbar comes
/// before the page). `None` when it cannot be read.
fn address(hwnd: HWND, engine: Engine) -> Option<String> {
    // SAFETY: COM is set up on this thread for the call and taken down after;
    // the interfaces are released when dropped, before that.
    unsafe {
        let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let read = || -> windows::core::Result<String> {
            let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            let window = automation.ElementFromHandle(windows::Win32::Foundation::HWND(hwnd))?;
            let condition = match engine {
                Engine::Gecko => automation.CreatePropertyCondition(UIA_AutomationIdPropertyId, &VARIANT::from(BSTR::from("urlbar-input")))?,
                Engine::Chromium => automation.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_EditControlTypeId.0))?,
            };
            let field = window.FindFirst(TreeScope_Descendants, &condition)?;
            // A field inside the page is the page's, whatever it says: with
            // the toolbar hidden (full screen, a pop-up) the first one can be.
            let walker = automation.ControlViewWalker()?;
            let mut parent = walker.GetParentElement(&field);
            while let Ok(element) = parent {
                if element.CurrentControlType()? == UIA_DocumentControlTypeId {
                    return Err(windows::core::Error::empty());
                }
                parent = walker.GetParentElement(&element);
            }
            let value: IUIAutomationValuePattern = field.GetCurrentPatternAs(UIA_ValuePatternId)?;
            Ok(value.CurrentValue()?.to_string())
        };
        let address = read().ok().filter(|a| !a.trim().is_empty());
        if com {
            CoUninitialize();
        }
        address
    }
}

/// Whether the window's program runs with more rights than PswManager (as
/// administrator): Windows then drops the keys typed into it (UIPI). One whose
/// rights cannot be read counts as such.
pub fn runs_higher(target: &Target) -> bool {
    // SAFETY: the handles are closed below.
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(target.hwnd(), &mut pid);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return true;
        }
        let theirs = integrity(process);
        CloseHandle(process);
        match (theirs, integrity(GetCurrentProcess())) {
            (Some(theirs), Some(ours)) => theirs > ours,
            _ => true,
        }
    }
}

/// The integrity level of a process (medium for a user's apps, high for one
/// run as administrator).
unsafe fn integrity(process: HANDLE) -> Option<u32> {
    let mut token: HANDLE = std::ptr::null_mut();
    if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
        return None;
    }
    let mut buffer = [0u64; 16]; // aligned for TOKEN_MANDATORY_LABEL and the SID after it
    let mut len = 0;
    let ok = GetTokenInformation(token, TokenIntegrityLevel, buffer.as_mut_ptr().cast(), std::mem::size_of_val(&buffer) as u32, &mut len) != 0;
    CloseHandle(token);
    if !ok {
        return None;
    }
    let sid = (*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid;
    let count = *GetSidSubAuthorityCount(sid);
    Some(*GetSidSubAuthority(sid, u32::from(count.checked_sub(1)?)))
}

/// Brings the window forward and types the steps into it, checking before
/// each key that it is still in front: if another window comes forward,
/// typing stops there.
pub fn type_into(target: &Target, steps: &[Step]) -> Result<(), String> {
    const MOVED: &str = "Another window came forward: Auto-Type stopped";
    if !target.is_current() {
        return Err("The window to type into is gone".into());
    }
    let hwnd = target.hwnd();
    // SAFETY: plain Win32 calls with the window's handle.
    let in_front = || unsafe { GetForegroundWindow() } == hwnd;
    if !in_front() {
        // SAFETY: as above.
        unsafe { SetForegroundWindow(hwnd) };
        if !wait_until(Duration::from_secs(1), in_front) {
            return Err("The window to type into could not be brought forward".into());
        }
    }
    // The hotkey's own keys must be up, or Ctrl+Alt would go with every key.
    if !wait_until(Duration::from_secs(3), || !modifiers_down()) {
        return Err("Let go of the hotkey's keys: Auto-Type did not type".into());
    }
    std::thread::sleep(Duration::from_millis(50));
    // SAFETY: the window's thread's keyboard layout, for the keys of characters.
    let layout = unsafe { GetKeyboardLayout(GetWindowThreadProcessId(hwnd, std::ptr::null_mut())) };
    let mut delay = KEY_DELAY;
    for step in steps {
        match step {
            Step::Text(text) => {
                let mut units = [0u16; 2];
                for c in text.chars() {
                    if !in_front() {
                        return Err(MOVED.into());
                    }
                    match c {
                        '\r' => continue,
                        '\n' => press(VK_RETURN, Mods::default()),
                        '\t' => press(VK_TAB, Mods::default()),
                        c => send(&c.encode_utf16(&mut units).iter().flat_map(|u| [unicode(*u, 0), unicode(*u, KEYEVENTF_KEYUP)]).collect::<Vec<_>>()),
                    }
                    pause(delay);
                }
            }
            Step::Key(key, mods) => {
                if !in_front() {
                    return Err(MOVED.into());
                }
                press(code(*key), *mods);
                pause(delay);
            }
            Step::Shortcut(c, mods) => {
                if !in_front() {
                    return Err(MOVED.into());
                }
                let mut units = [0u16; 2];
                let [unit] = c.encode_utf16(&mut units) else { return Err(format!("No key types {c}")) };
                // SAFETY: a plain lookup in the window's layout.
                let scan = unsafe { VkKeyScanExW(*unit, layout) };
                if scan == -1 {
                    return Err(format!("No key types {c} in this keyboard layout"));
                }
                let [vk, shift_state] = (scan as u16).to_le_bytes();
                let needed = Mods { shift: shift_state & 1 != 0, ctrl: shift_state & 2 != 0, alt: shift_state & 4 != 0, win: false };
                press(u16::from(vk), Mods { shift: mods.shift || needed.shift, ctrl: mods.ctrl || needed.ctrl, alt: mods.alt || needed.alt, win: mods.win });
                pause(delay);
            }
            Step::Pause(ms) => pause(*ms),
            Step::KeyDelay(ms) => delay = *ms,
        }
    }
    Ok(())
}

fn pause(ms: u32) {
    std::thread::sleep(Duration::from_millis(u64::from(ms)));
}

/// Waits up to `limit` for `done`, looking every 20 ms.
fn wait_until(limit: Duration, done: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while !done() {
        if start.elapsed() > limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

fn modifiers_down() -> bool {
    // SAFETY: GetAsyncKeyState takes any key code.
    [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN].iter().any(|vk| unsafe { GetAsyncKeyState(i32::from(*vk)) } as u16 & 0x8000 != 0)
}

/// The virtual key a [Key] presses.
fn code(key: Key) -> VIRTUAL_KEY {
    match key {
        Key::Tab => VK_TAB,
        Key::Enter => VK_RETURN,
        Key::Space => VK_SPACE,
        Key::Backspace => VK_BACK,
        Key::Delete => VK_DELETE,
        Key::Escape => VK_ESCAPE,
        Key::Insert => VK_INSERT,
        Key::Home => VK_HOME,
        Key::End => VK_END,
        Key::PageUp => VK_PRIOR,
        Key::PageDown => VK_NEXT,
        Key::Up => VK_UP,
        Key::Down => VK_DOWN,
        Key::Left => VK_LEFT,
        Key::Right => VK_RIGHT,
        Key::CapsLock => VK_CAPITAL,
        Key::F(n) => VK_F1 + VIRTUAL_KEY::from(n - 1),
    }
}

/// Presses `vk` with `mods` held, and lets go of all of them.
fn press(vk: VIRTUAL_KEY, mods: Mods) {
    let held: Vec<VIRTUAL_KEY> = [(mods.ctrl, VK_CONTROL), (mods.alt, VK_MENU), (mods.shift, VK_SHIFT), (mods.win, VK_LWIN)]
        .into_iter()
        .filter_map(|(on, vk)| on.then_some(vk))
        .collect();
    let mut inputs: Vec<INPUT> = held.iter().map(|m| virtual_key(*m, 0)).collect();
    inputs.push(virtual_key(vk, 0));
    inputs.push(virtual_key(vk, KEYEVENTF_KEYUP));
    inputs.extend(held.iter().rev().map(|m| virtual_key(*m, KEYEVENTF_KEYUP)));
    send(&inputs);
}

fn send(inputs: &[INPUT]) {
    // SAFETY: the inputs are whole INPUT structures.
    unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
}

fn keyboard(ki: KEYBDINPUT) -> INPUT {
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki } }
}

/// A key by its code; the keys of the navigation block are extended keys.
fn virtual_key(vk: VIRTUAL_KEY, flags: u32) -> INPUT {
    let extended = [VK_INSERT, VK_DELETE, VK_HOME, VK_END, VK_PRIOR, VK_NEXT, VK_UP, VK_DOWN, VK_LEFT, VK_RIGHT, VK_LWIN].contains(&vk);
    let flags = if extended { flags | KEYEVENTF_EXTENDEDKEY } else { flags };
    keyboard(KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 })
}

/// A UTF-16 unit typed as itself, whatever the keyboard layout.
fn unicode(unit: u16, flags: u32) -> INPUT {
    keyboard(KEYBDINPUT { wVk: 0, wScan: unit, dwFlags: KEYEVENTF_UNICODE | flags, time: 0, dwExtraInfo: 0 })
}
