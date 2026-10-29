//! The extension's host as the browsers find it: the native messaging
//! manifests in the app's data folder, and the registry keys that point to
//! them. Apart from the pipe, so a change to either does not risk the other.

use super::{wide, FIREFOX_EXTENSION, HOST_NAME};
use std::path::{Path, PathBuf};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE,
    REG_SZ, RRF_RT_REG_SZ,
};

/// The extension in the Chrome Web Store and in Edge Add-ons.
const CHROMIUM_ORIGINS: [&str; 2] = ["chrome-extension://pdffhmdngciaglkoonimfcmckehcpafo/", "chrome-extension://oboonakemofpalcgghocfoadofidjkkk/"];
/// Where each browser looks for the host, under `HKCU\Software`: Brave and
/// Vivaldi read Chrome's. The last is Firefox's, which needs its own manifest.
const BROWSER_KEYS: [&str; 4] = ["Google\\Chrome", "Chromium", "Microsoft\\Edge", "Mozilla"];
/// The manifests' description, which tells PswManager's apart from KeePassXC's.
const DESCRIPTION: &str = "PswManager for the KeePassXC-Browser extension";

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
