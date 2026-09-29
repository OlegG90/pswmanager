//! The database's encryption as the window shows and changes it: the cipher
//! and the key derivation (KDF) with its parameters, kept in the file's
//! header.

use keepass::config::{DatabaseConfig, KdfConfig, OuterCipherConfig};
use keepass::{Database, DatabaseKey};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Cipher {
    Aes256,
    ChaCha20,
    /// One the app keeps but does not offer (Twofish): only a database that
    /// has it keeps it.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kdf {
    Argon2id,
    Argon2d,
    AesKdf,
    /// One keepass-rs may add later: kept as it is, never chosen.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Encryption {
    pub cipher: Cipher,
    pub kdf: Kdf,
    /// Argon2's iterations, or AES-KDF's rounds.
    pub iterations: u64,
    /// Argon2 only: bytes of memory.
    pub memory: u64,
    /// Argon2 only: threads.
    pub parallelism: u32,
}

const MIB: u64 = 1 << 20;

/// The encryption a database has.
pub fn of(config: &DatabaseConfig) -> Encryption {
    let cipher = match config.outer_cipher_config {
        OuterCipherConfig::AES256 => Cipher::Aes256,
        OuterCipherConfig::ChaCha20 => Cipher::ChaCha20,
        _ => Cipher::Other,
    };
    let (kdf, iterations, memory, parallelism) = match &config.kdf_config {
        KdfConfig::Aes { rounds } => (Kdf::AesKdf, *rounds, 0, 0),
        KdfConfig::Argon2 { iterations, memory, parallelism, .. } => (Kdf::Argon2d, *iterations, *memory, *parallelism),
        KdfConfig::Argon2id { iterations, memory, parallelism, .. } => (Kdf::Argon2id, *iterations, *memory, *parallelism),
        _ => (Kdf::Other, 0, 0, 0),
    };
    Encryption { cipher, kdf, iterations, memory, parallelism }
}

/// Sets `config` to `wanted`, within the bounds a phone can still manage
/// (asking how long an unlock takes is [unlock_time]'s job); a cipher or KDF
/// the app does not offer only when `config` has it already.
pub fn apply(config: &mut DatabaseConfig, wanted: &Encryption) -> Result<(), String> {
    let now = of(config);
    config.outer_cipher_config = match wanted.cipher {
        Cipher::Aes256 => OuterCipherConfig::AES256,
        Cipher::ChaCha20 => OuterCipherConfig::ChaCha20,
        Cipher::Other if now.cipher == Cipher::Other => config.outer_cipher_config.clone(),
        Cipher::Other => return Err("Choose AES-256 or ChaCha20".into()),
    };
    let Encryption { iterations, memory, parallelism, .. } = *wanted;
    // The Argon2 version the database has, or keepass-rs's (1.3) after AES-KDF.
    let version = match (&config.kdf_config, DatabaseConfig::default().kdf_config) {
        (KdfConfig::Argon2 { version, .. } | KdfConfig::Argon2id { version, .. }, _) => *version,
        (_, KdfConfig::Argon2 { version, .. } | KdfConfig::Argon2id { version, .. }) => version,
        _ => unreachable!("keepass-rs defaults to Argon2"),
    };
    config.kdf_config = match wanted.kdf {
        Kdf::Other if now.kdf == Kdf::Other => config.kdf_config.clone(),
        Kdf::Other => return Err("Choose Argon2id, Argon2d or AES-KDF".into()),
        Kdf::AesKdf if (1..=1_000_000_000).contains(&iterations) => KdfConfig::Aes { rounds: iterations },
        Kdf::AesKdf => return Err("AES-KDF takes 1 to 1,000,000,000 rounds".into()),
        _ if !(1..=100).contains(&iterations) => return Err("Argon2 takes 1 to 100 iterations".into()),
        _ if !(MIB..=4096 * MIB).contains(&memory) => return Err("Argon2 takes 1 MiB to 4 GiB of memory".into()),
        _ if !(1..=64).contains(&parallelism) => return Err("Argon2 takes 1 to 64 threads".into()),
        Kdf::Argon2id => KdfConfig::Argon2id { iterations, memory, parallelism, version },
        Kdf::Argon2d => KdfConfig::Argon2 { iterations, memory, parallelism, version },
    };
    Ok(())
}

/// How long unlocking a database with `wanted` takes on this PC: an empty
/// one is written with it and opened again, the opening timed. As slow as
/// the unlock itself (and the writing before it).
pub fn unlock_time(wanted: &Encryption) -> Result<Duration, String> {
    let mut config = DatabaseConfig::default();
    apply(&mut config, &Encryption { cipher: Cipher::Aes256, ..wanted.clone() })?;
    let key = DatabaseKey::new().with_password("measure");
    let mut bytes = Vec::new();
    Database::with_config(config).save(&mut bytes, key.clone()).map_err(|e| format!("Cannot measure: {e}"))?;
    let start = Instant::now();
    Database::parse(&bytes, key).map_err(|e| format!("Cannot measure: {e}"))?;
    Ok(start.elapsed())
}

/// Keeps this device's encryption in a database from another device
/// (`theirs`), as the merge writes this device's file: the encryption goes
/// with the key. True when it changed anything.
pub fn keep(theirs: &mut DatabaseConfig, ours: &DatabaseConfig) -> bool {
    let changed = of(theirs) != of(ours);
    if changed {
        theirs.outer_cipher_config = ours.outer_cipher_config.clone();
        theirs.kdf_config = ours.kdf_config.clone();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argon2id(memory_mib: u64) -> Encryption {
        Encryption { cipher: Cipher::ChaCha20, kdf: Kdf::Argon2id, iterations: 2, memory: memory_mib * MIB, parallelism: 2 }
    }

    #[test]
    fn what_is_applied_reads_back() {
        let mut config = DatabaseConfig::default();
        for wanted in [argon2id(16), Encryption { kdf: Kdf::Argon2d, ..argon2id(8) }, Encryption { cipher: Cipher::Aes256, kdf: Kdf::AesKdf, iterations: 60_000, memory: 0, parallelism: 0 }] {
            apply(&mut config, &wanted).unwrap();
            assert_eq!(of(&config), wanted);
        }
    }

    #[test]
    fn out_of_bounds_and_a_new_twofish_are_refused() {
        let mut config = DatabaseConfig::default();
        assert!(apply(&mut config, &Encryption { iterations: 0, ..argon2id(16) }).is_err());
        assert!(apply(&mut config, &Encryption { memory: 1024, ..argon2id(16) }).is_err());
        assert!(apply(&mut config, &Encryption { parallelism: 0, ..argon2id(16) }).is_err());
        assert!(apply(&mut config, &Encryption { cipher: Cipher::Other, ..argon2id(16) }).is_err());
        // Twofish, which KeePassXC offers, is kept.
        config.outer_cipher_config = OuterCipherConfig::Twofish;
        apply(&mut config, &Encryption { cipher: Cipher::Other, ..argon2id(16) }).unwrap();
        assert_eq!(config.outer_cipher_config, OuterCipherConfig::Twofish);
    }

    #[test]
    fn an_unlock_is_timed() {
        let time = unlock_time(&argon2id(1)).unwrap();
        assert!(time > Duration::ZERO && time < Duration::from_secs(30), "{time:?}");
    }

    #[test]
    fn a_merge_keeps_this_devices_encryption() {
        let (mut theirs, mut ours) = (DatabaseConfig::default(), DatabaseConfig::default());
        apply(&mut ours, &argon2id(16)).unwrap();
        assert!(keep(&mut theirs, &ours));
        assert_eq!(of(&theirs), argon2id(16));
        assert!(!keep(&mut theirs, &ours));
    }
}
