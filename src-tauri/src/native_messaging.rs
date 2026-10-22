//! How the KeePassXC-Browser extension reaches PswManager (#273). The browser
//! starts the program its native messaging manifest names and talks to it over
//! stdin / stdout, each message a 4-byte length (little-endian) and JSON. That
//! program is `pswm.exe` itself, as a proxy: it hands the bytes on, unchanged,
//! to the tray app over a named pipe of this Windows user's, starting the app
//! (hidden, locked) when it is not running. The extension knows only
//! KeePassXC's host name, so PswManager registers under it, in place of
//! KeePassXC's registration.

use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, LocalFree, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::System::Console::{GetStdHandle, STD_INPUT_HANDLE};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PeekNamedPipe, WaitNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE,
    REG_SZ, RRF_RT_REG_SZ,
};

/// The name the extension asks the browser for: KeePassXC's.
const HOST_NAME: &str = "org.keepassxc.keepassxc_browser";
/// The extension in the Chrome Web Store and in Edge Add-ons.
const CHROMIUM_ORIGINS: [&str; 2] = ["chrome-extension://pdffhmdngciaglkoonimfcmckehcpafo/", "chrome-extension://oboonakemofpalcgghocfoadofidjkkk/"];
/// The extension in Firefox Add-ons.
const FIREFOX_EXTENSION: &str = "keepassxc-browser@keepassxc.org";
/// Where each browser looks for the host, under `HKCU\Software`: Brave and
/// Vivaldi read Chrome's. The last is Firefox's, which needs its own manifest.
const BROWSER_KEYS: [&str; 4] = ["Google\\Chrome", "Chromium", "Microsoft\\Edge", "Mozilla"];
/// The manifests' description, which tells PswManager's apart from KeePassXC's.
const DESCRIPTION: &str = "PswManager for the KeePassXC-Browser extension";
/// The largest message taken (the browser sends at most 4 GB, answers 1 MB).
const MAX_MESSAGE: usize = 1 << 20;

/// Whether this launch is a browser starting the host: Chromium browsers pass
/// the extension's origin first, Firefox the manifest and the extension's id.
pub fn started_by_browser(args: &[String]) -> bool {
    args.first().is_some_and(|a| a.starts_with("chrome-extension://")) || args.iter().any(|a| a == FIREFOX_EXTENSION)
}

/// The pipe of this Windows user's PswManager.
fn pipe_name() -> Vec<u16> {
    let user = std::env::var("USERNAME").unwrap_or_default();
    wide(&format!(r"\\.\pipe\pswmanager-browser-{user}"))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

// ---------------------------------------------------------------- the proxy

/// Runs as the browser's host: connects to the tray app (starting it when it
/// is not running) and hands the bytes on both ways until either side ends.
/// Both ends are looked at in turn rather than read on two threads: reads and
/// writes on one pipe handle wait for each other.
pub fn run_proxy() {
    let Some(mut pipe) = connect() else { return };
    // SAFETY: the process's own stdin, borrowed for its lifetime and never closed here.
    let stdin = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    let mut input = std::mem::ManuallyDrop::new(unsafe { File::from_raw_handle(stdin) });
    let mut output = std::io::stdout().lock();
    let mut buffer = vec![0u8; 64 << 10];
    loop {
        let (Some(from_browser), Some(from_app)) = (waiting(stdin), waiting(pipe.as_raw_handle())) else { return };
        if from_browser > 0 {
            let Ok(n) = input.read(&mut buffer) else { return };
            if n == 0 || pipe.write_all(&buffer[..n]).is_err() {
                return;
            }
        }
        if from_app > 0 {
            let Ok(n) = pipe.read(&mut buffer) else { return };
            if n == 0 || output.write_all(&buffer[..n]).and_then(|()| output.flush()).is_err() {
                return;
            }
        }
        if from_browser == 0 && from_app == 0 {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// How many bytes wait to be read on a pipe; `None` when its other end is gone.
fn waiting(pipe: HANDLE) -> Option<u32> {
    let mut bytes = 0u32;
    // SAFETY: only asks how many bytes wait on the pipe.
    let alive = unsafe { PeekNamedPipe(pipe, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut bytes, std::ptr::null_mut()) } != 0;
    alive.then_some(bytes)
}

/// The pipe to the tray app; the app is started (hidden, locked) when it does
/// not answer, and waited for up to 15 seconds.
fn connect() -> Option<File> {
    let name = pipe_name();
    let start = Instant::now();
    let mut started = false;
    loop {
        // SAFETY: the name is a NUL-terminated wide string; the handle becomes the File's.
        let handle = unsafe { CreateFileW(name.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
        if handle != INVALID_HANDLE_VALUE {
            return Some(unsafe { File::from_raw_handle(handle) });
        }
        if !started {
            started = true;
            let exe = std::env::current_exe().ok()?;
            // Not with the browser's stdin and stdout, which are this proxy's.
            let _ = std::process::Command::new(exe)
                .arg(crate::cli::AUTOSTART)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
        if start.elapsed() > Duration::from_secs(15) {
            return None;
        }
        // SAFETY: as above; waits for a free instance of a pipe that is there.
        unsafe { WaitNamedPipeW(name.as_ptr(), 200) };
        std::thread::sleep(Duration::from_millis(200));
    }
}

// ---------------------------------------------------------------- the app's side

/// What the tray app keeps: a way to tell every connected extension that the
/// database locked or unlocked.
#[derive(Default)]
pub struct Clients(Mutex<Vec<Sender<serde_json::Value>>>);

impl Clients {
    /// Tells every connected extension `message`; ones gone are forgotten.
    pub fn tell(&self, message: serde_json::Value) {
        self.0.lock().unwrap().retain(|client| client.send(message.clone()).is_ok());
    }
}

/// Serves the pipe on a thread of its own: a connection each, answered by
/// `answer` (a fresh protocol session per connection) while `allowed` says so.
pub fn serve(clients: &'static Clients, failed: impl FnOnce() + Send + 'static, allowed: impl Fn() -> bool + Send + Sync + Clone + 'static, answer: impl Fn(&mut crate::browser::Session, &serde_json::Value) -> serde_json::Value + Send + Sync + Clone + 'static) {
    std::thread::spawn(move || {
        let name = pipe_name();
        let Some(security) = this_user_only() else {
            eprintln!("Cannot serve the browser extension: no security for the pipe");
            return failed();
        };
        let mut first = true;
        loop {
            // The first instance claims the name: no other program may serve it.
            let flags = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
            // SAFETY: the name is a NUL-terminated wide string; the security
            // attributes live as long as the thread.
            let pipe = unsafe {
                CreateNamedPipeW(name.as_ptr(), flags, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, 64 << 10, 64 << 10, 0, &security.attributes)
            };
            if pipe == INVALID_HANDLE_VALUE {
                eprintln!("Cannot serve the browser extension: the pipe is taken");
                return failed();
            }
            first = false;
            // SAFETY: waits for a client on the pipe just made.
            let connected = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) } != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
            if !connected || !allowed() {
                // SAFETY: the handle is ours and not used again.
                unsafe { CloseHandle(pipe) };
                continue;
            }
            let (tell, told) = mpsc::channel();
            clients.0.lock().unwrap().push(tell);
            let (allowed, answer) = (allowed.clone(), answer.clone());
            // SAFETY: the handle is this connection's; the File closes it.
            let file = unsafe { File::from_raw_handle(pipe) };
            std::thread::spawn(move || converse(file, told, allowed, answer));
        }
    });
}

/// One extension's connection: its messages answered in turn, and what the
/// app tells it in between. Looked at every 50 ms rather than read in a
/// blocking call, which would hold the pipe from being written to.
fn converse(
    mut file: File,
    told: Receiver<serde_json::Value>,
    allowed: impl Fn() -> bool,
    answer: impl Fn(&mut crate::browser::Session, &serde_json::Value) -> serde_json::Value,
) {
    let pipe = file.as_raw_handle() as HANDLE;
    let mut session = crate::browser::Session::default();
    loop {
        match told.try_recv() {
            Ok(message) if write_message(&mut file, &message).is_err() => return,
            Err(TryRecvError::Disconnected) => return,
            _ => {}
        }
        let Some(bytes) = waiting(pipe).filter(|_| allowed()) else { return };
        if bytes < 4 {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        let Ok(request) = read_message(&mut file) else { return };
        let reply = answer(&mut session, &request);
        if write_message(&mut file, &reply).is_err() {
            return;
        }
    }
}

fn read_message(file: &mut File) -> std::io::Result<serde_json::Value> {
    let mut length = [0u8; 4];
    file.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_MESSAGE {
        return Err(std::io::Error::other("message too long"));
    }
    let mut body = vec![0u8; length];
    file.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(std::io::Error::other)
}

fn write_message(file: &mut File, message: &serde_json::Value) -> std::io::Result<()> {
    let body = message.to_string();
    file.write_all(&(body.len() as u32).to_le_bytes())?;
    file.write_all(body.as_bytes())?;
    file.flush()
}

/// Security that lets only this Windows user (and the system) open the pipe:
/// no other user, not even to read it.
struct ThisUserOnly {
    attributes: SECURITY_ATTRIBUTES,
}

// SAFETY: the descriptor is only read, by CreateNamedPipeW, on the serving thread.
unsafe impl Send for ThisUserOnly {}

impl Drop for ThisUserOnly {
    fn drop(&mut self) {
        // SAFETY: the descriptor came from LocalAlloc in the conversion below.
        unsafe { LocalFree(self.attributes.lpSecurityDescriptor) };
    }
}

fn this_user_only() -> Option<ThisUserOnly> {
    // SAFETY: the token is closed below; the SID string and the descriptor
    // are freed with LocalFree; the buffer is as long as said.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut buffer = [0u64; 64];
        let mut len = 0;
        let read = GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast(), std::mem::size_of_val(&buffer) as u32, &mut len) != 0;
        CloseHandle(token);
        if !read {
            return None;
        }
        let user = (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let mut text: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user, &mut text) == 0 {
            return None;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, (0..).take_while(|&i| *text.add(i) != 0).count()));
        LocalFree(text.cast());
        let sddl = wide(&format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})"));
        let mut descriptor = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, std::ptr::null_mut()) == 0 {
            return None;
        }
        let attributes = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 };
        Some(ThisUserOnly { attributes })
    }
}

// ---------------------------------------------------------------- registration

/// The manifests, in the app's data folder: Chromium's and Firefox's.
fn manifests(data_dir: &Path) -> (PathBuf, PathBuf) {
    let dir = data_dir.join("browser");
    (dir.join(format!("{HOST_NAME}.chromium.json")), dir.join(format!("{HOST_NAME}.firefox.json")))
}

/// Registers `exe` as the extension's host in every browser: the manifests,
/// and the registry keys that point to them (written again at each start,
/// so a portable copy that moved is found).
pub fn register(data_dir: &Path, exe: &Path) -> Result<(), String> {
    let (chromium, firefox) = manifests(data_dir);
    std::fs::create_dir_all(chromium.parent().expect("in a folder")).map_err(|e| format!("Cannot write the browser manifest: {e}"))?;
    let manifest = |allowed: serde_json::Value| {
        let mut m = serde_json::json!({ "name": HOST_NAME, "description": DESCRIPTION, "path": exe, "type": "stdio" });
        m.as_object_mut().expect("an object").extend(allowed.as_object().expect("an object").clone());
        serde_json::to_string_pretty(&m).expect("JSON")
    };
    let write = |path: &Path, text: String| std::fs::write(path, text).map_err(|e| format!("Cannot write {}: {e}", path.display()));
    write(&chromium, manifest(serde_json::json!({ "allowed_origins": CHROMIUM_ORIGINS })))?;
    write(&firefox, manifest(serde_json::json!({ "allowed_extensions": [FIREFOX_EXTENSION] })))?;
    for browser in BROWSER_KEYS {
        let file = if browser == "Mozilla" { &firefox } else { &chromium };
        set_default(&host_key(browser), &file.display().to_string())?;
    }
    Ok(())
}

/// Whether a manifest is PswManager's: one this copy wrote, or one a copy
/// elsewhere (a portable one that moved) wrote, by its description.
fn is_ours(manifest: &Path, data_dir: &Path) -> bool {
    let (chromium, firefox) = manifests(data_dir);
    manifest == chromium || manifest == firefox || std::fs::read_to_string(manifest).is_ok_and(|text| text.contains(DESCRIPTION))
}

/// Removes PswManager's registration, leaving another program's (KeePassXC's) alone.
pub fn unregister(data_dir: &Path) {
    for browser in BROWSER_KEYS {
        let key = host_key(browser);
        if registered(&key).is_some_and(|path| is_ours(&path, data_dir)) {
            let key = wide(&key);
            // SAFETY: a NUL-terminated wide key name under HKCU.
            unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, key.as_ptr()) };
        }
    }
}

/// Whether another program (KeePassXC) is registered for the extension in a
/// browser: turning PswManager's on replaces it, and KeePassXC takes it back
/// when it starts with its browser integration on.
pub fn other_registered(data_dir: &Path) -> bool {
    BROWSER_KEYS.iter().filter_map(|b| registered(&host_key(b))).any(|path| !is_ours(&path, data_dir))
}

fn host_key(browser: &str) -> String {
    format!("Software\\{browser}\\NativeMessagingHosts\\{HOST_NAME}")
}

/// The manifest a browser key points to, if it is there.
fn registered(key: &str) -> Option<PathBuf> {
    let key = wide(key);
    let mut buffer = [0u16; 1024];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: reads the key's default value into the buffer, as long as said.
    let read = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), std::ptr::null(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size) };
    if read != 0 {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1);
    Some(PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
}

fn set_default(key: &str, value: &str) -> Result<(), String> {
    let (key_name, value) = (wide(key), wide(value));
    let mut handle: HKEY = std::ptr::null_mut();
    // SAFETY: creates or opens the key under HKCU and writes its default value; the key is closed after.
    unsafe {
        if RegCreateKeyExW(HKEY_CURRENT_USER, key_name.as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, std::ptr::null(), &mut handle, std::ptr::null_mut()) != 0 {
            return Err(format!("Cannot register with the browsers ({key})"));
        }
        let written = RegSetValueExW(handle, std::ptr::null(), 0, REG_SZ, value.as_ptr().cast(), (value.len() * 2) as u32);
        RegCloseKey(handle);
        if written != 0 {
            return Err(format!("Cannot register with the browsers ({key})"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsers_start_the_host_with_these_arguments() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(started_by_browser(&args(&["chrome-extension://pdffhmdngciaglkoonimfcmckehcpafo/", "--parent-window=0"])));
        assert!(started_by_browser(&args(&[r"C:\x\org.keepassxc.keepassxc_browser.firefox.json", "keepassxc-browser@keepassxc.org"])));
        assert!(!started_by_browser(&args(&[r"C:\db.kdbx"])));
        assert!(!started_by_browser(&args(&["--autostart"])));
    }
}
