//! How the KeePassXC-Browser extension reaches PswManager (#273). The browser
//! starts the program its native messaging manifest names and talks to it over
//! stdin / stdout, each message a 4-byte length (little-endian) and JSON. That
//! program is `pswm.exe` itself, as a proxy: it hands the bytes on, unchanged,
//! to the tray app over a named pipe of this Windows user's, starting the app
//! (hidden, locked) when it is not running. The extension knows only
//! KeePassXC's host name, so PswManager registers under it, in place of
//! KeePassXC's registration.
//!
//! The pipe is named by the user's SID, and the proxy talks only to a server
//! running as the same user: another account on the PC that made a pipe of
//! that name first gets nothing from the browser.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, LocalFree, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT};
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER};
use windows_sys::Win32::System::RemoteDesktop::{WTSEnumerateProcessesExW, WTSFreeMemoryExW, WTSTypeProcessInfoLevel1, WTS_CURRENT_SERVER_HANDLE, WTS_PROCESS_INFO_EXW};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::System::Console::{GetStdHandle, STD_INPUT_HANDLE};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerProcessId, PeekNamedPipe, WaitNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

mod registration;

pub use registration::{other_registered, register, unregister};

/// The name the extension asks the browser for: KeePassXC's.
const HOST_NAME: &str = "org.keepassxc.keepassxc_browser";
/// The extension in Firefox Add-ons.
const FIREFOX_EXTENSION: &str = "keepassxc-browser@keepassxc.org";
/// The largest message taken (the browser sends at most 4 GB, answers 1 MB).
const MAX_MESSAGE: usize = 1 << 20;
/// How long a message may take to come whole once it started: a client that
/// stops halfway is let go, rather than holding the connection.
const MESSAGE_WITHIN: Duration = Duration::from_secs(10);
/// How the proxy opens the pipe: its server may learn who connected, not act as them.
const IDENTIFICATION_ONLY: u32 = SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION;
/// The processes of every Windows session, to `WTSEnumerateProcessesExW`.
const WTS_ANY_SESSION: u32 = 0xFFFF_FFFE;

/// Whether this launch is a browser starting the host: Chromium browsers pass
/// the extension's origin first, Firefox the manifest and the extension's id.
pub fn started_by_browser(args: &[String]) -> bool {
    args.first().is_some_and(|a| a.starts_with("chrome-extension://")) || args.iter().any(|a| a == FIREFOX_EXTENSION)
}

/// The pipe of the PswManager of the Windows user with SID `user`.
fn pipe_name(user: &str) -> Vec<u16> {
    wide(&format!(r"\\.\pipe\pswmanager-browser-{user}"))
}

/// The SID of the Windows user this process runs as (`S-1-5-21-...`).
fn this_user() -> Option<String> {
    // SAFETY: the token is closed below; the buffer is as long as said.
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
        sid_text((*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid)
    }
}

/// A SID in its string form.
///
/// # Safety
/// `sid` must point to a valid SID.
unsafe fn sid_text(sid: PSID) -> Option<String> {
    // SAFETY: the string is NUL-terminated and freed with LocalFree.
    unsafe {
        let mut text: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(sid, &mut text) == 0 {
            return None;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, (0..).take_while(|&i| *text.add(i) != 0).count()));
        LocalFree(text.cast());
        Some(sid)
    }
}

/// Whether the program serving the pipe runs as `user`. Its user comes from
/// the list of processes rather than its token, which a program run as
/// administrator does not let this one read.
fn served_by(pipe: HANDLE, user: &str) -> bool {
    let mut id = 0;
    // SAFETY: asks for the server's process id, then for the processes with
    // their users (Task Manager's list), freed after.
    unsafe {
        if GetNamedPipeServerProcessId(pipe, &mut id) == 0 {
            return false;
        }
        let (mut level, mut list, mut count) = (1u32, std::ptr::null_mut(), 0u32);
        if WTSEnumerateProcessesExW(WTS_CURRENT_SERVER_HANDLE, &mut level, WTS_ANY_SESSION, &mut list, &mut count) == 0 {
            return false;
        }
        let processes = std::slice::from_raw_parts(list.cast::<WTS_PROCESS_INFO_EXW>(), count as usize);
        let owner = processes.iter().find(|p| p.ProcessId == id).filter(|p| !p.pUserSid.is_null()).and_then(|p| sid_text(p.pUserSid));
        WTSFreeMemoryExW(WTSTypeProcessInfoLevel1, list.cast(), count);
        owner.as_deref() == Some(user)
    }
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
        let (Some(from_browser), Some(from_app)) = (waiting_on(stdin), waiting_on(pipe.as_raw_handle())) else { return };
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
fn waiting_on(pipe: HANDLE) -> Option<u32> {
    let mut bytes = 0u32;
    // SAFETY: only asks how many bytes wait on the pipe.
    let alive = unsafe { PeekNamedPipe(pipe, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut bytes, std::ptr::null_mut()) } != 0;
    alive.then_some(bytes)
}

/// The pipe to the tray app; the app is started (hidden, locked) when it does
/// not answer, and waited for up to 15 seconds. A pipe another user's program
/// serves is left at once; that program may only learn who connected, not act
/// as them (identification only).
fn connect() -> Option<File> {
    let user = this_user()?;
    let name = pipe_name(&user);
    let start = Instant::now();
    let mut started = false;
    loop {
        // SAFETY: the name is a NUL-terminated wide string; the handle becomes the File's.
        let handle = unsafe { CreateFileW(name.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, std::ptr::null(), OPEN_EXISTING, IDENTIFICATION_ONLY, std::ptr::null_mut()) };
        if handle != INVALID_HANDLE_VALUE {
            let pipe = unsafe { File::from_raw_handle(handle) };
            if !served_by(handle, &user) {
                eprintln!("The browser pipe is served by another Windows user's program");
                return None;
            }
            return Some(pipe);
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
pub struct Clients {
    next: AtomicU64,
    connected: Mutex<HashMap<u64, Sender<serde_json::Value>>>,
}

impl Clients {
    /// Tells every connected extension `message`; ones gone are forgotten.
    pub fn tell(&self, message: serde_json::Value) {
        self.connected.lock().unwrap().retain(|_, client| client.send(message.clone()).is_ok());
    }

    /// A new connection, and what it is told.
    fn join(&'static self) -> (Joined, Receiver<serde_json::Value>) {
        let (tell, told) = mpsc::channel();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.connected.lock().unwrap().insert(id, tell);
        (Joined { clients: self, id }, told)
    }
}

/// A connection among the [Clients], forgotten when it ends.
struct Joined {
    clients: &'static Clients,
    id: u64,
}

impl Drop for Joined {
    fn drop(&mut self) {
        self.clients.connected.lock().unwrap().remove(&self.id);
    }
}

/// Serves the pipe on a thread of its own: a connection each, answered by
/// `answer` (a fresh protocol session per connection) while `allowed` says so.
pub fn serve(clients: &'static Clients, failed: impl FnOnce() + Send + 'static, allowed: impl Fn() -> bool + Send + Sync + Clone + 'static, answer: impl Fn(&mut crate::browser::Session, &serde_json::Value) -> serde_json::Value + Send + Sync + Clone + 'static) {
    std::thread::spawn(move || {
        let Some(user) = this_user() else {
            eprintln!("Cannot serve the browser extension: no Windows user");
            return failed();
        };
        let name = pipe_name(&user);
        let Some(security) = this_user_only(&user) else {
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
            let (joined, told) = clients.join();
            let (allowed, answer) = (allowed.clone(), answer.clone());
            // SAFETY: the handle is this connection's; the File closes it.
            let file = unsafe { File::from_raw_handle(pipe) };
            std::thread::spawn(move || {
                let _among_clients = joined;
                converse(file, told, allowed, answer, MESSAGE_WITHIN);
            });
        }
    });
}

/// The bytes of one connection both ways, and how many wait to be read: the
/// pipe, or bytes in memory in the tests, so [converse] runs without one.
trait Connection: Read + Write {
    /// How many bytes wait to be read; `None` when the other end is gone.
    fn waiting(&self) -> Option<u32>;
}

impl Connection for File {
    fn waiting(&self) -> Option<u32> {
        waiting_on(self.as_raw_handle() as HANDLE)
    }
}

/// One extension's connection: its messages answered in turn, and what the
/// app tells it in between, also while an answer waits for the user. Looked
/// at every 50 ms, and only the bytes there read, rather than read in a
/// blocking call, which would hold the pipe from being written to. A message
/// must come whole `within` once it started ([MESSAGE_WITHIN]).
fn converse(
    mut connection: impl Connection,
    told: Receiver<serde_json::Value>,
    allowed: impl Fn() -> bool,
    answer: impl Fn(&mut crate::browser::Session, &serde_json::Value) -> serde_json::Value + Sync,
    within: Duration,
) {
    let mut session = crate::browser::Session::default();
    let mut incoming = Vec::new();
    // When the message being read began to come.
    let mut begun = Instant::now();
    loop {
        if !deliver(&mut connection, &told) {
            return;
        }
        let Some(bytes) = connection.waiting().filter(|_| allowed()) else { return };
        if bytes > 0 {
            if incoming.is_empty() {
                begun = Instant::now();
            }
            let start = incoming.len();
            incoming.resize(start + bytes as usize, 0);
            if connection.read_exact(&mut incoming[start..]).is_err() {
                return;
            }
        }
        let request = match next_message(&mut incoming) {
            Some(Ok(request)) => request,
            Some(Err(_)) => return,
            None if !incoming.is_empty() && begun.elapsed() > within => return,
            None => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        begun = Instant::now();
        // Answered on a thread of its own, which may wait for the user; none
        // when the connection went meanwhile.
        let reply = std::thread::scope(|scope| {
            let (done, answered) = mpsc::channel();
            let (session, request, answer) = (&mut session, &request, &answer);
            scope.spawn(move || done.send(answer(session, request)));
            let mut alive = true;
            loop {
                match answered.recv_timeout(Duration::from_millis(50)) {
                    Ok(reply) => return alive.then_some(reply),
                    Err(RecvTimeoutError::Timeout) => alive = alive && deliver(&mut connection, &told),
                    Err(RecvTimeoutError::Disconnected) => return None,
                }
            }
        });
        if reply.is_none_or(|reply| write_message(&mut connection, &reply).is_err()) {
            return;
        }
    }
}

/// Writes what the app told the connection since; false when it is gone.
fn deliver(connection: &mut impl Write, told: &Receiver<serde_json::Value>) -> bool {
    loop {
        match told.try_recv() {
            Ok(message) if write_message(connection, &message).is_err() => return false,
            Ok(_) => {}
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
}

/// Takes the first message off the bytes come so far, once it is whole.
fn next_message(incoming: &mut Vec<u8>) -> Option<std::io::Result<serde_json::Value>> {
    let length: [u8; 4] = incoming.get(..4)?.try_into().ok()?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_MESSAGE {
        return Some(Err(std::io::Error::other("message too long")));
    }
    let message = serde_json::from_slice(incoming.get(4..4 + length)?).map_err(std::io::Error::other);
    incoming.drain(..4 + length);
    Some(message)
}

fn write_message(connection: &mut impl Write, message: &serde_json::Value) -> std::io::Result<()> {
    let body = message.to_string();
    connection.write_all(&(body.len() as u32).to_le_bytes())?;
    connection.write_all(body.as_bytes())?;
    connection.flush()
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

/// [ThisUserOnly] for the user with SID `user`.
fn this_user_only(user: &str) -> Option<ThisUserOnly> {
    // SAFETY: the descriptor is freed with LocalFree when dropped.
    unsafe {
        let sddl = wide(&format!("D:P(A;;GA;;;SY)(A;;GA;;;{user})"));
        let mut descriptor = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, std::ptr::null_mut()) == 0 {
            return None;
        }
        let attributes = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 };
        Some(ThisUserOnly { attributes })
    }
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

    #[test]
    fn a_message_is_taken_once_it_is_whole() {
        let mut incoming = [framed(r#"{"action":"a"}"#), framed(r#"{"action":"b"}"#)].concat();
        let second = incoming.split_off(incoming.len() - 3);
        assert_eq!(next_message(&mut incoming).unwrap().unwrap()["action"], "a");
        assert!(next_message(&mut incoming).is_none(), "the second has not all come");
        incoming.extend(second);
        assert_eq!(next_message(&mut incoming).unwrap().unwrap()["action"], "b");
        assert!(incoming.is_empty() && next_message(&mut incoming).is_none());
        let mut long = ((MAX_MESSAGE + 1) as u32).to_le_bytes().to_vec();
        assert!(next_message(&mut long).unwrap().is_err());
    }

    #[test]
    fn the_pipe_is_this_user_s_and_its_server_is_checked() {
        let user = this_user().unwrap();
        assert!(user.starts_with("S-1-"));
        assert!(String::from_utf16_lossy(&pipe_name(&user)).ends_with(&format!("pswmanager-browser-{user}\0")));
        let security = this_user_only(&user).unwrap();
        let name = wide(&format!(r"\\.\pipe\pswmanager-test-{}", std::process::id()));
        // SAFETY: a pipe of this test's, and a client of it; both closed by their Files.
        let (server, client) = unsafe {
            let server = CreateNamedPipeW(name.as_ptr(), PIPE_ACCESS_DUPLEX, PIPE_TYPE_BYTE | PIPE_WAIT, 1, 4096, 4096, 0, &security.attributes);
            assert_ne!(server, INVALID_HANDLE_VALUE);
            let client = CreateFileW(name.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, std::ptr::null(), OPEN_EXISTING, IDENTIFICATION_ONLY, std::ptr::null_mut());
            assert_ne!(client, INVALID_HANDLE_VALUE);
            (File::from_raw_handle(server), File::from_raw_handle(client))
        };
        assert!(served_by(client.as_raw_handle(), &user));
        assert!(!served_by(client.as_raw_handle(), "S-1-5-18"), "not the system's");
        drop(server);
    }

    /// A connection in memory: what the extension sends waits in `incoming`,
    /// what the app writes lands in `written`.
    #[derive(Clone, Default)]
    struct Memory {
        incoming: std::sync::Arc<Mutex<std::collections::VecDeque<u8>>>,
        written: std::sync::Arc<Mutex<Vec<u8>>>,
        gone: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl Read for Memory {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let mut incoming = self.incoming.lock().unwrap();
            let n = buffer.len().min(incoming.len());
            for (to, from) in buffer.iter_mut().zip(incoming.drain(..n)) {
                *to = from;
            }
            Ok(n)
        }
    }

    impl Write for Memory {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.written.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Connection for Memory {
        fn waiting(&self) -> Option<u32> {
            let gone = self.gone.load(Ordering::Relaxed);
            (!gone).then(|| self.incoming.lock().unwrap().len() as u32)
        }
    }

    impl Memory {
        fn send(&self, bytes: &[u8]) {
            self.incoming.lock().unwrap().extend(bytes);
        }

        /// The messages the app wrote so far.
        fn replies(&self) -> Vec<serde_json::Value> {
            let mut written = self.written.lock().unwrap().clone();
            std::iter::from_fn(|| next_message(&mut written).map(Result::unwrap)).collect()
        }
    }

    fn framed(body: &str) -> Vec<u8> {
        [&(body.len() as u32).to_le_bytes()[..], body.as_bytes()].concat()
    }

    /// A conversation on its own thread, while `allowed` says so, answering
    /// each request with its action; the action `slow` takes a while to
    /// answer, as one waiting for the user does.
    fn conversation_while(allowed: impl Fn() -> bool + Send + 'static, within: Duration) -> (Memory, Sender<serde_json::Value>, std::thread::JoinHandle<()>) {
        let (memory, (tell, told)) = (Memory::default(), mpsc::channel());
        let connection = memory.clone();
        let answer = |_: &mut crate::browser::Session, request: &serde_json::Value| {
            if request["action"] == "slow" {
                std::thread::sleep(Duration::from_millis(300));
            }
            serde_json::json!({ "answered": request["action"] })
        };
        let ended = std::thread::spawn(move || converse(connection, told, allowed, answer, within));
        (memory, tell, ended)
    }

    fn conversation(within: Duration) -> (Memory, Sender<serde_json::Value>, std::thread::JoinHandle<()>) {
        conversation_while(|| true, within)
    }

    /// Waits up to two seconds for `done`.
    fn soon(done: impl Fn() -> bool) -> bool {
        let start = Instant::now();
        while !done() {
            if start.elapsed() > Duration::from_secs(2) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        true
    }

    #[test]
    fn a_message_in_pieces_is_answered_once_whole() {
        let (memory, _tell, ended) = conversation(MESSAGE_WITHIN);
        let message = framed(r#"{"action":"a"}"#);
        memory.send(&message[..3]);
        std::thread::sleep(Duration::from_millis(150));
        assert!(memory.replies().is_empty(), "not whole yet");
        memory.send(&message[3..]);
        assert!(soon(|| memory.replies() == [serde_json::json!({ "answered": "a" })]));
        memory.send(&[framed(r#"{"action":"b"}"#), framed(r#"{"action":"c"}"#)].concat());
        assert!(soon(|| memory.replies().len() == 3), "two at once, answered in turn");
        assert_eq!(memory.replies()[2]["answered"], "c");
        assert!(!ended.is_finished());
    }

    #[test]
    fn what_the_app_tells_goes_out_while_an_answer_waits() {
        let (memory, tell, _ended) = conversation(MESSAGE_WITHIN);
        memory.send(&framed(r#"{"action":"slow"}"#));
        // Told once the request was taken, so while its answer waits.
        assert!(soon(|| memory.incoming.lock().unwrap().is_empty()));
        std::thread::sleep(Duration::from_millis(100));
        tell.send(serde_json::json!({ "action": "database-locked" })).unwrap();
        assert!(soon(|| memory.replies().len() == 2));
        assert_eq!(memory.replies()[0]["action"], "database-locked", "before the answer");
        assert_eq!(memory.replies()[1]["answered"], "slow");
    }

    #[test]
    fn a_message_that_stops_halfway_ends_the_connection() {
        let (memory, _tell, ended) = conversation(Duration::from_millis(200));
        memory.send(&framed(r#"{"action":"a"}"#)[..6]);
        assert!(soon(|| ended.is_finished()));
        assert!(memory.replies().is_empty());
    }

    #[test]
    fn a_broken_message_ends_the_connection() {
        let (memory, _tell, ended) = conversation(MESSAGE_WITHIN);
        memory.send(&framed("not JSON"));
        assert!(soon(|| ended.is_finished()));
        let (memory, _tell, ended) = conversation(MESSAGE_WITHIN);
        memory.send(&((MAX_MESSAGE + 1) as u32).to_le_bytes());
        assert!(soon(|| ended.is_finished()), "too long, before its body comes");
        assert!(memory.replies().is_empty());
    }

    #[test]
    fn the_connection_ends_when_the_extension_or_the_app_lets_go() {
        let (memory, _tell, ended) = conversation(MESSAGE_WITHIN);
        memory.gone.store(true, Ordering::Relaxed);
        assert!(soon(|| ended.is_finished()), "the extension went");
        let (_memory, tell, ended) = conversation(MESSAGE_WITHIN);
        drop(tell);
        assert!(soon(|| ended.is_finished()), "the app let go of the connection");
        let allowed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let still = allowed.clone();
        let (_memory, _tell, ended) = conversation_while(move || still.load(Ordering::Relaxed), MESSAGE_WITHIN);
        std::thread::sleep(Duration::from_millis(100));
        assert!(!ended.is_finished());
        allowed.store(false, Ordering::Relaxed);
        assert!(soon(|| ended.is_finished()), "the extension was turned off");
    }

    #[test]
    fn a_connection_that_ends_is_forgotten() {
        let clients: &'static Clients = Box::leak(Box::default());
        let (joined, told) = clients.join();
        clients.tell(serde_json::json!({ "action": "database-locked" }));
        assert_eq!(told.try_recv().unwrap()["action"], "database-locked");
        drop(joined);
        assert!(clients.connected.lock().unwrap().is_empty());
    }
}
