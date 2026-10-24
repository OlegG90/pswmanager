//! Auto-Type on Windows (#272): the window the hotkey was pressed in, the
//! address a browser there shows (read through UI Automation), and the core's
//! steps typed into it with `SendInput`.

use pswm_core::autotype::{Key, Mods, Step, Window};
use pswm_core::browser::Page;
use std::time::{Duration, Instant};
use windows::core::BSTR;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationValuePattern, TreeScope_Descendants, UIA_AutomationIdPropertyId, UIA_DocumentControlTypeId,
    UIA_ControlTypePropertyId, UIA_EditControlTypeId, UIA_ValuePatternId,
};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, RECT};
use windows_sys::Win32::Security::{GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel, TOKEN_MANDATORY_LABEL, TOKEN_QUERY};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardLayout, MapVirtualKeyW, SendInput, VkKeyScanExW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, MAPVK_VK_TO_VSC,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1,
    VK_HOME, VK_INSERT, VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
};

/// Browsers made from Chromium, by their program's file name: one of them is
/// a browser even when its address cannot be read (and then nothing fits
/// it). Apps made with Electron share Chromium's window class but show no
/// address, so the class alone cannot tell.
const BROWSERS: [&str; 14] = [
    "chrome.exe", "msedge.exe", "brave.exe", "vivaldi.exe", "opera.exe", "browser.exe", "chromium.exe", "thorium.exe", "whale.exe", "arc.exe",
    "iridium.exe", "slimjet.exe", "maxthon.exe", "yandex.exe",
];
/// Programs with Firefox's window class that are no browser: every other
/// window of that class is one (Firefox, LibreWolf, Zen, Floorp, Waterfox…).
const GECKO_APPS: [&str; 2] = ["thunderbird.exe", "betterbird.exe"];
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

    /// Where the window is on screen, in pixels: its left, top, right and bottom.
    pub fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: the handle may be gone, which the call reports.
        (unsafe { GetWindowRect(self.hwnd(), &mut rect) } != 0).then_some((rect.left, rect.top, rect.right, rect.bottom))
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
        let program = program(pid);
        let gecko_browser = engine == Some(Engine::Gecko) && !program.as_deref().is_some_and(|p| GECKO_APPS.contains(&p));
        let known_browser = gecko_browser || program.as_deref().is_some_and(|p| BROWSERS.contains(&p));
        // On a browser's first call (a cold start) UI Automation may not have
        // its tree yet: a known browser is asked again a few times. An app
        // made with Electron shows no address ever, so it is not.
        let mut read = engine.and_then(|e| address(hwnd, e));
        for _ in 0..3 {
            if read.is_some() || !known_browser {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
            read = engine.and_then(|e| address(hwnd, e));
        }
        // A browser not in the list counts as one when it shows an address:
        // its title, which the page chooses, must never pick the entry.
        let browser = read.is_some() || known_browser;
        let page = read.as_deref().and_then(Page::of_address);
        Some(Target { hwnd: hwnd as usize, engine, window: Window { title, browser, page }, since: Instant::now() })
    }
}

/// The window that comes forward once PswManager's stepped aside: waits up
/// to half a second for Windows to bring it forward.
pub fn behind() -> Option<Target> {
    let mut found = None;
    wait_until(Duration::from_millis(500), || {
        found = foreground();
        found.is_some()
    });
    found
}

/// The page the target's browser shows now; `None` when it is no browser or
/// its address cannot be read.
fn page_now(target: &Target) -> Option<Page> {
    target.engine.and_then(|e| address(target.hwnd(), e)).as_deref().and_then(Page::of_address)
}

/// Whether the target still shows the page it showed when the hotkey was
/// pressed (a browser may have gone on: a redirect after Enter, another tab);
/// any other window always does.
pub fn on_same_page(target: &Target) -> bool {
    if !target.window.browser {
        return true;
    }
    match (page_now(target), &target.window.page) {
        (Some(now), Some(then)) => now.same_place(then),
        (now, then) => now.is_none() && then.is_none(),
    }
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
/// each key that it is still in front, and before each step that a browser
/// still shows the same page: otherwise typing stops there.
pub fn type_into(target: &Target, steps: &[Step]) -> Result<(), String> {
    const MOVED: &str = "Another window came forward: Auto-Type stopped";
    const PAGE: &str = "The browser went on to another page: Auto-Type stopped";
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
    // Every shortcut's key must be in that layout (`^a` in a Ukrainian one is
    // not): checked before the first key, rather than stopping half-way.
    for step in steps {
        if let Step::Shortcut(c, _) = step {
            shortcut_key(*c, layout)?;
        }
    }
    let mut delay = KEY_DELAY;
    for step in steps {
        // Every key goes to the window only while it is in front; a text's
        // before each of its characters too.
        if !matches!(step, Step::Pause(_) | Step::KeyDelay(_)) {
            if !in_front() {
                return Err(MOVED.into());
            }
            if !on_same_page(target) {
                return Err(PAGE.into());
            }
        }
        match step {
            Step::Text(text) => {
                for c in text.chars() {
                    if !in_front() {
                        return Err(MOVED.into());
                    }
                    match c {
                        '\r' => continue,
                        '\n' => press(VK_RETURN, Mods::default()),
                        '\t' => press(VK_TAB, Mods::default()),
                        c => type_char(c),
                    }
                    pause(delay);
                }
            }
            Step::Key(key, mods) => {
                press(code(*key), *mods);
                pause(delay);
            }
            Step::Shortcut(c, mods) => {
                let (vk, needed) = shortcut_key(*c, layout)?;
                press(vk, mods.with(needed));
                pause(delay);
            }
            Step::Pause(ms) => pause(*ms),
            Step::KeyDelay(ms) => delay = *ms,
        }
    }
    Ok(())
}

/// The key that types `c` in `layout`, and the modifiers the layout needs for it.
fn shortcut_key(c: char, layout: windows_sys::Win32::UI::Input::KeyboardAndMouse::HKL) -> Result<(VIRTUAL_KEY, Mods), String> {
    let mut units = [0u16; 2];
    let [unit] = c.encode_utf16(&mut units) else { return Err(format!("No key types {c}")) };
    // SAFETY: a plain lookup in the window's layout.
    let scan = unsafe { VkKeyScanExW(*unit, layout) };
    if scan == -1 {
        return Err(format!("No key types {c} in this keyboard layout: Auto-Type did not type"));
    }
    let [vk, shift_state] = (scan as u16).to_le_bytes();
    Ok((u16::from(vk), Mods { shift: shift_state & 1 != 0, ctrl: shift_state & 2 != 0, alt: shift_state & 4 != 0, win: false }))
}

/// Types one character as Unicode; the inputs that held it are wiped after.
fn type_char(c: char) {
    let mut units = [0u16; 2];
    let units = c.encode_utf16(&mut units);
    let mut inputs = [unicode(0, 0); 4];
    for (n, unit) in units.iter().enumerate() {
        inputs[2 * n] = unicode(*unit, 0);
        inputs[2 * n + 1] = unicode(*unit, KEYEVENTF_KEYUP);
    }
    send(&inputs[..2 * units.len()]);
    // SAFETY: an INPUT and a u16 are plain data; written through volatile
    // pointers so the wiping is not left out as unused.
    for input in &mut inputs {
        unsafe { std::ptr::write_volatile(input, unicode(0, 0)) };
    }
    for unit in units.iter_mut() {
        unsafe { std::ptr::write_volatile(unit, 0) };
    }
}

fn pause(ms: u32) {
    std::thread::sleep(Duration::from_millis(u64::from(ms)));
}

/// Waits up to `limit` for `done`, looking every 20 ms.
fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
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
    // With its scan code too: remote desktops, Java and some games read only that.
    // SAFETY: a plain lookup.
    let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) } as u16;
    keyboard(KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 })
}

/// A UTF-16 unit typed as itself, whatever the keyboard layout.
fn unicode(unit: u16, flags: u32) -> INPUT {
    keyboard(KEYBDINPUT { wVk: 0, wScan: unit, dwFlags: KEYEVENTF_UNICODE | flags, time: 0, dwExtraInfo: 0 })
}
