//! Secrets the app keeps between runs (a cloud account's refresh token), in
//! the Windows Credential Manager: protected for the Windows user, never in
//! the state file.

use std::ptr;
use windows_sys::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};
use zeroize::Zeroizing;

/// `pswmanager:<name>`, as the Credential Manager lists it.
fn target(name: &str) -> Vec<u16> {
    format!("pswmanager:{name}").encode_utf16().chain([0]).collect()
}

pub fn write(name: &str, secret: &str) -> Result<(), String> {
    let mut target = target(name);
    let mut blob = Zeroizing::new(secret.as_bytes().to_vec());
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // SAFETY: every pointer in `credential` points into buffers alive for the call.
    if unsafe { CredWriteW(&credential, 0) } == 0 {
        return Err(format!("Cannot store the sign-in: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

/// The secret, or `None` when there is none.
pub fn read(name: &str) -> Option<Zeroizing<String>> {
    let target = target(name);
    let mut credential: *mut CREDENTIALW = ptr::null_mut();
    // SAFETY: `target` is NUL-terminated; on success `credential` is freed below.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
        return None;
    }
    // SAFETY: CredReadW succeeded, so `credential` points to a valid CREDENTIALW
    // whose blob holds `CredentialBlobSize` bytes.
    let secret = unsafe {
        let c = &*credential;
        let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
        let secret = Zeroizing::new(String::from_utf8_lossy(bytes).into_owned());
        CredFree(credential.cast());
        secret
    };
    Some(secret)
}

pub fn delete(name: &str) {
    let target = target(name);
    // SAFETY: `target` is NUL-terminated. A missing credential is fine.
    unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_kept_read_and_removed() {
        let name = format!("test-{}", std::process::id());
        assert!(read(&name).is_none());
        write(&name, "refresh-token").unwrap();
        assert_eq!(read(&name).unwrap().as_str(), "refresh-token");
        delete(&name);
        assert!(read(&name).is_none());
    }
}
