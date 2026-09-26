//! Tells the app when Windows locks or the session is switched away: a hidden
//! window registered for session notifications, on its own thread.

use std::sync::OnceLock;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW, MSG, WM_WTSSESSION_CHANGE,
    WNDCLASSW, WTS_CONSOLE_DISCONNECT, WTS_REMOTE_DISCONNECT, WTS_SESSION_LOCK,
};

static ON_LEAVE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Calls `on_leave` whenever the session is locked or disconnected. Call once.
pub fn watch(on_leave: impl Fn() + Send + Sync + 'static) {
    if ON_LEAVE.set(Box::new(on_leave)).is_err() {
        return;
    }
    std::thread::spawn(|| {
        // SAFETY: plain Win32 calls on this thread; the class name outlives the
        // window, and the message loop runs until the process ends.
        unsafe {
            let class_name: Vec<u16> = "PswManagerSessionWatch\0".encode_utf16().collect();
            let instance = GetModuleHandleW(std::ptr::null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class_name.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&class);
            // A hidden top-level window: message-only windows get no session notifications.
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() || WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) == 0 {
                return;
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                DispatchMessageW(&msg);
            }
        }
    });
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_WTSSESSION_CHANGE
        && matches!(wparam as u32, WTS_SESSION_LOCK | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT)
    {
        if let Some(on_leave) = ON_LEAVE.get() {
            on_leave();
        }
    }
    // SAFETY: forwarding the arguments Windows gave us.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
