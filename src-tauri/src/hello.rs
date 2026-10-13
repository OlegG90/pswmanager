//! Unlocking with Windows Hello (#235): after an unlock with the master
//! password, the password is sealed for that database so that only this
//! Windows user can open it, and only once Windows Hello confirms (face,
//! fingerprint or PIN). It is encrypted with DPAPI, with the signature of a
//! random challenge by the user's Windows Hello key as the entropy: that key
//! never leaves the TPM and signs only after Windows Hello asks the user, and
//! its RSA signature of the same challenge is always the same. The challenge,
//! the encrypted password and when it was sealed are kept in the Credential
//! Manager, one per database.

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::core::{Array, HSTRING};
use windows::Security::Credentials::{KeyCredential, KeyCredentialCreationOption, KeyCredentialManager, KeyCredentialStatus};
use windows::Security::Cryptography::CryptographicBuffer;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB};
use zeroize::Zeroizing;

/// The user's Windows Hello key for the app, shared by every database.
const KEY_NAME: &str = "PswManager";
/// The Credential Manager's entries: `pswmanager:hello:<database file>`.
const PREFIX: &str = "hello:";

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Why the sealed password could not be had.
pub enum Failure {
    /// The user closed Windows Hello's prompt.
    Cancelled,
    /// Nothing sealed (any more) for this database.
    NotSealed,
    Other(String),
}

/// What is kept for a database.
#[derive(Serialize, Deserialize)]
struct Sealed {
    /// Base64.
    challenge: String,
    /// The password encrypted with DPAPI, base64; empty for none.
    password: String,
    /// When it was sealed (seconds since 1970): the master password is asked
    /// for again after the set days.
    sealed: u64,
}

fn name(database: &Path) -> String {
    format!("{PREFIX}{}", database.display())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn read(database: &Path) -> Option<Sealed> {
    serde_json::from_str(&crate::credentials::read(&name(database))?).ok()
}

/// Whether this PC has Windows Hello set up for the user.
pub fn available() -> bool {
    KeyCredentialManager::IsSupportedAsync().and_then(|r| r.join()).unwrap_or(false)
}

/// Whether a password is sealed for `database` that is not older than `every`.
pub fn ready(database: &Path, every: Duration) -> bool {
    read(database).is_some_and(|s| now().saturating_sub(s.sealed) < every.as_secs())
}

/// Seals `password` for `database`; Windows Hello asks the user (the first
/// time, to set up the app's key).
pub fn seal(database: &Path, password: Option<&str>) -> Result<(), String> {
    let key = open_key(true)?.ok_or("Windows Hello is not set up")?;
    let mut challenge = [0u8; 32];
    getrandom::fill(&mut challenge).map_err(|e| e.to_string())?;
    let entropy = sign(&key, &challenge)?.ok_or("Cancelled")?;
    let password = match password {
        Some(password) => B64.encode(protect(password.as_bytes(), &entropy)?),
        None => String::new(),
    };
    let sealed = Sealed { challenge: B64.encode(challenge), password, sealed: now() };
    crate::credentials::write(&name(database), &serde_json::to_string(&sealed).map_err(|e| e.to_string())?)
}

/// The password sealed for `database`, once Windows Hello confirms; `None`
/// for a database without one (a key file only).
pub fn unseal(database: &Path) -> Result<Option<Zeroizing<String>>, Failure> {
    let sealed = read(database).ok_or(Failure::NotSealed)?;
    let Some(key) = open_key(false).map_err(Failure::Other)? else {
        forget(database);
        return Err(Failure::NotSealed);
    };
    let challenge = B64.decode(&sealed.challenge).map_err(|e| Failure::Other(e.to_string()))?;
    let entropy = sign(&key, &challenge).map_err(Failure::Other)?.ok_or(Failure::Cancelled)?;
    if sealed.password.is_empty() {
        return Ok(None);
    }
    let encrypted = B64.decode(&sealed.password).map_err(|e| Failure::Other(e.to_string()))?;
    let password = unprotect(&encrypted, &entropy).map_err(|_| {
        // Another key now (Windows Hello was set up again): this one is no use.
        forget(database);
        Failure::NotSealed
    })?;
    String::from_utf8(password.to_vec()).map(|p| Some(Zeroizing::new(p))).map_err(|e| Failure::Other(e.to_string()))
}

/// What was sealed for `database` goes (a new key, or the database removed).
pub fn forget(database: &Path) {
    crate::credentials::delete(&name(database));
}

/// Every sealed password goes, and the app's Windows Hello key with them
/// (Windows Hello unlock turned off).
pub fn forget_all() {
    crate::credentials::delete_all(PREFIX);
    if let Ok(deleting) = KeyCredentialManager::DeleteAsync(&HSTRING::from(KEY_NAME)) {
        let _ = deleting.join();
    }
}

/// The app's Windows Hello key; with `create`, made when there is none (Windows
/// Hello asks the user). `None` when there is none, or the user declined.
fn open_key(create: bool) -> Result<Option<KeyCredential>, String> {
    let name = HSTRING::from(KEY_NAME);
    let failed = |e: windows::core::Error| format!("Windows Hello: {}", e.message());
    let opened = KeyCredentialManager::OpenAsync(&name).and_then(|r| r.join()).map_err(failed)?;
    let result = match opened.Status().map_err(failed)? {
        KeyCredentialStatus::NotFound if create => {
            let creating = KeyCredentialManager::RequestCreateAsync(&name, KeyCredentialCreationOption::FailIfExists).map_err(failed)?;
            to_front();
            creating.join().map_err(failed)?
        }
        _ => opened,
    };
    match result.Status().map_err(failed)? {
        KeyCredentialStatus::Success => result.Credential().map(Some).map_err(failed),
        KeyCredentialStatus::NotFound | KeyCredentialStatus::UserCanceled => Ok(None),
        status => Err(format!("Windows Hello did not open the key (status {})", status.0)),
    }
}

/// `data` signed with the key, once Windows Hello confirms; `None` when the
/// user cancelled.
fn sign(key: &KeyCredential, data: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    let failed = |e: windows::core::Error| format!("Windows Hello: {}", e.message());
    let buffer = CryptographicBuffer::CreateFromByteArray(data).map_err(failed)?;
    let signing = key.RequestSignAsync(&buffer).map_err(failed)?;
    to_front();
    let result = signing.join().map_err(failed)?;
    match result.Status().map_err(failed)? {
        KeyCredentialStatus::Success => {
            let mut signature = Array::<u8>::new();
            CryptographicBuffer::CopyToByteArray(&result.Result().map_err(failed)?, &mut signature).map_err(failed)?;
            Ok(Some(Zeroizing::new(signature.to_vec())))
        }
        KeyCredentialStatus::UserCanceled => Ok(None),
        status => Err(format!("Windows Hello did not sign (status {})", status.0)),
    }
}

/// Windows Hello's prompt opens behind the app's window when another app had
/// the focus last (a desktop app cannot give it a parent window): it is
/// brought to the front as soon as it shows, for a few seconds at most.
fn to_front() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow};
    std::thread::spawn(|| {
        let class: Vec<u16> = "Credential Dialog Xaml Host".encode_utf16().chain([0]).collect();
        for _ in 0..50 {
            // SAFETY: `class` is NUL-terminated; a missing window is null.
            let window = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
            if !window.is_null() {
                // SAFETY: a window handle FindWindowW just returned.
                unsafe { SetForegroundWindow(window) };
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
}

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr().cast_mut() }
}

/// Encrypts `data` for this Windows user, with `entropy` needed to decrypt it.
fn protect(data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: the input blobs point into slices alive for the call; `out` is
    // filled by the call and freed with LocalFree below.
    let done = unsafe { CryptProtectData(&blob(data), std::ptr::null(), &blob(entropy), std::ptr::null(), std::ptr::null(), 0, &mut out) };
    if done == 0 {
        return Err(format!("Cannot encrypt the password: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: on success `out` holds `cbData` bytes allocated by the system.
    let encrypted = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    // SAFETY: allocated by CryptProtectData, freed once.
    unsafe { LocalFree(out.pbData.cast()) };
    Ok(encrypted)
}

/// Decrypts what [protect] encrypted with the same `entropy`.
fn unprotect(data: &[u8], entropy: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: as in `protect`.
    let done = unsafe { CryptUnprotectData(&blob(data), std::ptr::null_mut(), &blob(entropy), std::ptr::null(), std::ptr::null(), 0, &mut out) };
    if done == 0 {
        return Err(format!("Cannot decrypt the password: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: on success `out` holds `cbData` bytes allocated by the system;
    // they are wiped before they are freed.
    let password = unsafe {
        let bytes = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
        let password = Zeroizing::new(bytes.to_vec());
        zeroize::Zeroize::zeroize(bytes);
        LocalFree(out.pbData.cast());
        password
    };
    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpapi_needs_the_same_entropy() {
        let encrypted = protect(b"master password", b"signature").unwrap();
        assert_eq!(unprotect(&encrypted, b"signature").unwrap().as_slice(), b"master password");
        assert!(unprotect(&encrypted, b"another signature").is_err());
    }
}
