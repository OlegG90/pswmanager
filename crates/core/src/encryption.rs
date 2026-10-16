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
    Aes,
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

/// What a new database gets: AES-256 and Argon2id tuned like KeePassXC's
/// default (64 MiB), so a phone opens it in a few seconds.
pub const DEFAULT: Encryption = Encryption { cipher: Cipher::Aes256, kdf: Kdf::Argon2id, iterations: 10, memory: 64 * MIB, parallelism: 2 };

/// The encryption a database has.
pub fn of(config: &DatabaseConfig) -> Encryption {
    let cipher = match config.outer_cipher_config {
        OuterCipherConfig::AES256 => Cipher::Aes256,
        OuterCipherConfig::ChaCha20 => Cipher::ChaCha20,
        _ => Cipher::Other,
    };
    let (kdf, iterations, memory, parallelism) = match &config.kdf_config {
        KdfConfig::Aes { rounds } => (Kdf::Aes, *rounds, 0, 0),
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
        Kdf::Aes if (1..=1_000_000_000).contains(&iterations) => KdfConfig::Aes { rounds: iterations },
        Kdf::Aes => return Err("AES-KDF takes 1 to 1,000,000,000 rounds".into()),
        _ if !(1..=100).contains(&iterations) => return Err("Argon2 takes 1 to 100 iterations".into()),
        _ if !(MIB..=4096 * MIB).contains(&memory) => return Err("Argon2 takes 1 MiB to 4 GiB of memory".into()),
        _ if !(1..=64).contains(&parallelism) => return Err("Argon2 takes 1 to 64 threads".into()),
        Kdf::Argon2id => KdfConfig::Argon2id { iterations, memory, parallelism, version },
        Kdf::Argon2d => KdfConfig::Argon2 { iterations, memory, parallelism, version },
    };
    Ok(())
}

/// How long unlocking a database with `wanted` takes on this PC: an empty one
/// is written with it and the writing timed, which derives the key as an
/// unlock does (the rest of an empty file takes next to nothing).
pub fn unlock_time(wanted: &Encryption) -> Result<Duration, String> {
    let mut config = DatabaseConfig::default();
    apply(&mut config, &Encryption { cipher: Cipher::Aes256, ..wanted.clone() })?;
    let start = Instant::now();
    Database::with_config(config)
        .save(&mut Vec::new(), DatabaseKey::new().with_password("measure"))
        .map_err(|e| format!("Cannot measure: {e}"))?;
    Ok(start.elapsed())
}

/// Keeps this device's encryption in a database from another device
/// (`theirs`) that a merge writes: the encryption goes with the key, so it
/// stays this device's unless the other device changed its key later
/// (`MasterKeyChanged`). True when it changed anything.
pub fn keep_ours(theirs: &mut Database, ours: &Database) -> bool {
    if theirs.meta.master_key_changed > ours.meta.master_key_changed || of(&theirs.config) == of(&ours.config) {
        return false;
    }
    theirs.config.outer_cipher_config = ours.config.outer_cipher_config.clone();
    theirs.config.kdf_config = ours.config.kdf_config.clone();
    true
}

/// Why a database file is not opened: its header asks for a key derivation
/// beyond the ceilings below ([check_header]).
pub const TOO_COSTLY: &str = "This database's key derivation asks for more memory or time than PswManager allows (Argon2 \
     at most 4 GiB of memory), so it is not opened: lower it in KeePassXC if the file is yours, or the file was damaged or replaced";

// The most key derivation a file may ask for. Nothing in the header is
// checked until the key has been derived with it, so a file anyone replaced
// could otherwise make each open run out of memory (the app aborts) or never
// finish. The ceilings are this app's own strongest settings ([apply]), and
// leave room for what KeePass, Keepass2Android and KeePassXC make, their
// "1 second" (KeePassXC: up to 5 s) benchmarks included, which give low
// memory thousands of iterations.
/// Argon2 memory, as [apply] allows (KeePass and Keepass2Android stop at 2 GiB).
const MAX_MEMORY: u64 = 4096 * MIB;
/// Argon2 threads, which rust-argon2 starts all at once: KeePassXC offers
/// up to 128.
const MAX_LANES: u64 = 256;
/// Argon2 iterations times memory, the work of filling it: [apply]'s 100
/// iterations of 4 GiB.
const MAX_WORK: u64 = 100 * MAX_MEMORY;
/// Argon2 iterations times threads: rust-argon2 starts a thread for each
/// lane four times an iteration, which costs more than the work when memory
/// is low.
const MAX_THREAD_STARTS: u64 = 1 << 20;
/// AES-KDF rounds, as [apply] and KeePassXC allow.
const MAX_AES_ROUNDS: u64 = 1_000_000_000;

// The key derivations' ids in a KDBX 4 header, as keepass-rs knows them.
const ARGON2D: [u8; 16] = [0xef, 0x63, 0x6d, 0xdf, 0x8c, 0x29, 0x44, 0x4b, 0x91, 0xf7, 0xa9, 0xa4, 0x03, 0xe3, 0x0a, 0x0c];
const ARGON2ID: [u8; 16] = [0x9e, 0x29, 0x8b, 0x19, 0x56, 0xdb, 0x47, 0x73, 0xb2, 0x3d, 0xfc, 0x3e, 0xc6, 0xf0, 0xa1, 0xe6];
const AES_KDBX3: [u8; 16] = [0xc9, 0xd9, 0xf3, 0x9a, 0x62, 0x8a, 0x44, 0x60, 0xbf, 0x74, 0x0d, 0x08, 0xc1, 0x8a, 0x4f, 0xea];
const AES_KDBX4: [u8; 16] = [0x7c, 0x02, 0xbb, 0x82, 0x79, 0xa7, 0x4a, 0xc0, 0x92, 0x7d, 0x11, 0x4a, 0x00, 0x64, 0x82, 0x38];

/// Refuses a database file whose header asks for more key derivation than
/// the ceilings above ([TOO_COSTLY]), before keepass-rs derives the key from
/// it. Reads the header as keepass-rs 0.15 does (KeePass 1, KDBX 3 and 4); a
/// header it cannot read is left to keepass-rs, which refuses it before
/// deriving anything.
pub fn check_header(file: &[u8]) -> Result<(), String> {
    const KDBX: [u8; 4] = [0x03, 0xd9, 0xa2, 0x9a];
    if file.get(..4) != Some(&KDBX[..]) {
        return Ok(());
    }
    let too_costly = match (le(file, 4, 4), le(file, 10, 2)) {
        // KeePass 1: AES-KDF, its rounds at a fixed place.
        (Some(0xb54b_fb65), _) => le(file, 120, 4).is_some_and(|rounds| rounds > MAX_AES_ROUNDS),
        // KDBX 3: AES-KDF, its rounds in entry 6.
        (Some(0xb54b_fb67), Some(3)) => header_entries(file, 2).any(|(kind, value)| kind == 6 && le(value, 0, 8).is_some_and(|rounds| rounds > MAX_AES_ROUNDS)),
        // KDBX 4: the parameters in entry 11. keepass-rs takes the last such
        // entry and fails on any it cannot read, so each one is checked.
        (Some(0xb54b_fb67), Some(4)) => header_entries(file, 4).any(|(kind, value)| kind == 11 && kdf_too_costly(value)),
        _ => false,
    };
    if too_costly {
        return Err(TOO_COSTLY.into());
    }
    Ok(())
}

/// The outer header's entries after the 12 bytes of signature and version:
/// a type, a length of `length_size` bytes, and that many bytes, up to the
/// end entry (type 0) or the first that does not fit.
fn header_entries(file: &[u8], length_size: usize) -> impl Iterator<Item = (u8, &[u8])> {
    let mut at = 12;
    std::iter::from_fn(move || {
        let kind = *file.get(at)?;
        let start = at.checked_add(1 + length_size)?;
        let end = start.checked_add(usize::try_from(le(file, at + 1, length_size)?).ok()?)?;
        let value = file.get(start..end)?;
        at = end;
        (kind != 0).then_some((kind, value))
    })
}

/// True when KDBX 4 KDF parameters (a VariantDictionary) ask for more than
/// the ceilings. Read as keepass-rs reads them: a name given twice counts
/// with its last value, and parameters it would not take are left to it.
fn kdf_too_costly(dictionary: &[u8]) -> bool {
    const U32: u8 = 0x04;
    const U64: u8 = 0x05;
    const BYTES: u8 = 0x42;
    if le(dictionary, 0, 2) != Some(0x100) {
        return false;
    }
    // The last value (its type and bytes) of each name that matters.
    let (mut uuid, mut memory, mut iterations, mut lanes, mut rounds) = (None, None, None, None, None);
    let mut at = 2;
    while at + 9 < dictionary.len() {
        let Some((kind, name, value, next)) = dictionary_item(dictionary, at) else { return false };
        // The value types keepass-rs knows; it refuses any other.
        if ![U32, U64, 0x08, 0x0c, 0x0d, 0x18, BYTES].contains(&kind) {
            return false;
        }
        match name {
            b"$UUID" => uuid = Some((kind, value)),
            b"M" => memory = Some((kind, value)),
            b"I" => iterations = Some((kind, value)),
            b"P" => lanes = Some((kind, value)),
            b"R" => rounds = Some((kind, value)),
            _ => {}
        }
        at = next;
    }
    if dictionary.get(at) != Some(&0) {
        return false;
    }
    let number = |item: Option<(u8, &[u8])>, wanted: u8, size: usize| item.filter(|(kind, _)| *kind == wanted).and_then(|(_, value)| le(value, 0, size));
    match uuid.filter(|(kind, _)| *kind == BYTES).map(|(_, value)| value) {
        Some(uuid) if uuid == ARGON2D || uuid == ARGON2ID => {
            let (Some(memory), Some(iterations), Some(lanes)) = (number(memory, U64, 8), number(iterations, U64, 8), number(lanes, U32, 4)) else {
                return false;
            };
            memory > MAX_MEMORY || lanes > MAX_LANES || iterations.saturating_mul(memory) > MAX_WORK || iterations.saturating_mul(lanes) > MAX_THREAD_STARTS
        }
        Some(uuid) if uuid == AES_KDBX3 || uuid == AES_KDBX4 => number(rounds, U64, 8).is_some_and(|rounds| rounds > MAX_AES_ROUNDS),
        _ => false,
    }
}

/// The VariantDictionary item at `at`: its value type, name and value, and
/// where the next one starts.
fn dictionary_item(dictionary: &[u8], at: usize) -> Option<(u8, &[u8], &[u8], usize)> {
    let kind = *dictionary.get(at)?;
    let name_start = at + 5;
    let name_end = name_start.checked_add(usize::try_from(le(dictionary, at + 1, 4)?).ok()?)?;
    let name = dictionary.get(name_start..name_end)?;
    let value_start = name_end.checked_add(4)?;
    let value_end = value_start.checked_add(usize::try_from(le(dictionary, name_end, 4)?).ok()?)?;
    let value = dictionary.get(value_start..value_end)?;
    Some((kind, name, value, value_end))
}

/// The little-endian number of `size` bytes (at most 8) at `at`, if the
/// bytes are there.
fn le(bytes: &[u8], at: usize, size: usize) -> Option<u64> {
    let bytes = bytes.get(at..at.checked_add(size)?)?;
    Some(bytes.iter().rev().fold(0, |n, &b| (n << 8) | u64::from(b)))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use sha2::Digest;

    fn argon2id(memory_mib: u64) -> Encryption {
        Encryption { cipher: Cipher::ChaCha20, kdf: Kdf::Argon2id, iterations: 2, memory: memory_mib * MIB, parallelism: 2 }
    }

    #[test]
    fn what_is_applied_reads_back() {
        let mut config = DatabaseConfig::default();
        for wanted in [argon2id(16), Encryption { kdf: Kdf::Argon2d, ..argon2id(8) }, Encryption { cipher: Cipher::Aes256, kdf: Kdf::Aes, iterations: 60_000, memory: 0, parallelism: 0 }] {
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
    fn a_merge_keeps_this_devices_encryption_unless_the_other_key_is_newer() {
        let (mut theirs, mut ours) = (Database::new(), Database::new());
        apply(&mut ours.config, &argon2id(16)).unwrap();
        let other = theirs.clone();
        assert!(keep_ours(&mut theirs, &ours));
        assert_eq!(of(&theirs.config), argon2id(16));
        assert!(!keep_ours(&mut theirs, &ours));

        // The other device changed its key later: its encryption goes with it.
        let mut theirs = other;
        theirs.meta.master_key_changed = Some(keepass::db::Times::now());
        assert!(!keep_ours(&mut theirs, &ours));
        assert_ne!(of(&theirs.config), argon2id(16));
    }

    /// `file`, a KDBX 4 file keepass-rs wrote with Argon2, made to ask for
    /// 2^32 + 1 iterations, its header hash made right again. No KeePass app
    /// writes that; keepass-rs would cut it to 1 iteration, so a test that
    /// opens it does not hang if the header check is not there.
    pub fn too_costly(file: &[u8]) -> Vec<u8> {
        let mut file = file.to_vec();
        let item = [0x05, 1, 0, 0, 0, b'I', 8, 0, 0, 0];
        let at = file.windows(item.len()).position(|w| w == item).unwrap() + item.len();
        file[at..at + 8].copy_from_slice(&((1u64 << 32) + 1).to_le_bytes());
        let mut end = 12;
        loop {
            let (kind, length) = (file[end], u32::from_le_bytes(file[end + 1..end + 5].try_into().unwrap()));
            end += 5 + length as usize;
            if kind == 0 {
                break;
            }
        }
        let hash: [u8; 32] = sha2::Sha256::digest(&file[..end]).into();
        file[end..end + 32].copy_from_slice(&hash);
        file
    }

    /// A KDBX 4 file, as far as [check_header] reads it: a header with these
    /// KDF parameters (type, name, value).
    fn kdbx4(params: &[(u8, &str, &[u8])]) -> Vec<u8> {
        let mut dictionary = vec![0x00, 0x01];
        for (kind, name, value) in params {
            dictionary.push(*kind);
            dictionary.extend((name.len() as u32).to_le_bytes());
            dictionary.extend(name.as_bytes());
            dictionary.extend((value.len() as u32).to_le_bytes());
            dictionary.extend(*value);
        }
        dictionary.push(0);
        let mut file = vec![0x03, 0xd9, 0xa2, 0x9a, 0x67, 0xfb, 0x4b, 0xb5, 0x01, 0x00, 0x04, 0x00];
        for (kind, value) in [(2, &[0; 16][..]), (11, &dictionary), (0, &[0; 4][..])] {
            file.push(kind);
            file.extend((value.len() as u32).to_le_bytes());
            file.extend(value);
        }
        file
    }

    fn argon2(memory: u64, iterations: u64, lanes: u32) -> Vec<u8> {
        kdbx4(&[(0x42, "$UUID", &ARGON2ID), (0x05, "M", &memory.to_le_bytes()), (0x05, "I", &iterations.to_le_bytes()), (0x04, "P", &lanes.to_le_bytes())])
    }

    fn aes(rounds: u64) -> Vec<u8> {
        kdbx4(&[(0x42, "$UUID", &AES_KDBX4), (0x05, "R", &rounds.to_le_bytes())])
    }

    #[test]
    fn a_key_derivation_any_keepass_app_sets_is_let_through() {
        let fine = [
            ("the default", argon2(64 * MIB, 10, 2)),
            ("this app's strongest", argon2(4096 * MIB, 100, 64)),
            ("a benchmark on little memory", argon2(MIB, 50_000, 2)),
            ("KeePassXC's most threads", argon2(1024 * MIB, 20, 128)),
            ("AES-KDF at KeePassXC's most", aes(1_000_000_000)),
        ];
        for (what, file) in fine {
            assert_eq!(check_header(&file), Ok(()), "{what}");
        }
        // As keepass-rs writes them.
        let mut written = Vec::new();
        Database::new().save(&mut written, DatabaseKey::new().with_password("pw")).unwrap();
        assert_eq!(check_header(&written), Ok(()));
        assert_eq!(check_header(&too_costly(&written)), Err(TOO_COSTLY.to_string()));
    }

    #[test]
    fn a_key_derivation_no_keepass_app_sets_is_refused() {
        let refused = [
            ("terabytes of memory", argon2(4 << 40, 2, 2)),
            ("billions of iterations", argon2(64 * MIB, 4_000_000_000, 2)),
            ("half a million threads", argon2(4096 * MIB, 1, 1 << 19)),
            ("more work than this app's strongest", argon2(4096 * MIB, 101, 1)),
            ("iterations that start too many threads", argon2(16 << 10, 1 << 20, 2)),
            ("AES-KDF for ever", aes(1 << 63)),
        ];
        for (what, file) in refused {
            assert_eq!(check_header(&file), Err(TOO_COSTLY.to_string()), "{what}");
        }
    }

    #[test]
    fn kdbx_3_and_keepass_1_rounds_are_checked_too() {
        let kdbx3 = |rounds: u64| {
            let mut file = vec![0x03, 0xd9, 0xa2, 0x9a, 0x67, 0xfb, 0x4b, 0xb5, 0x01, 0x00, 0x03, 0x00, 6, 8, 0];
            file.extend(rounds.to_le_bytes());
            file.extend([0, 0, 0]);
            file
        };
        let keepass1 = |rounds: u32| {
            let mut file = vec![0x03, 0xd9, 0xa2, 0x9a, 0x65, 0xfb, 0x4b, 0xb5];
            file.resize(120, 0);
            file.extend(rounds.to_le_bytes());
            file
        };
        assert_eq!(check_header(&kdbx3(60_000)), Ok(()));
        assert!(check_header(&kdbx3(1 << 63)).is_err());
        assert_eq!(check_header(&keepass1(60_000)), Ok(()));
        assert!(check_header(&keepass1(u32::MAX)).is_err());
    }

    #[test]
    fn a_header_that_does_not_read_is_left_to_keepass() {
        // Cut anywhere: refused only once the KDF parameters are all there.
        let file = argon2(4 << 40, 2, 2);
        let parameters_end = file.len() - 9;
        for length in 0..file.len() {
            assert_eq!(check_header(&file[..length]).is_err(), length >= parameters_end, "{length}");
        }
        // Parameters keepass-rs does not take: it refuses them itself.
        let mistyped = kdbx4(&[(0x42, "$UUID", &ARGON2ID), (0x04, "M", &u32::MAX.to_le_bytes()), (0x05, "I", &u64::MAX.to_le_bytes()), (0x04, "P", &1u32.to_le_bytes())]);
        let unknown_type = kdbx4(&[(0x42, "$UUID", &AES_KDBX4), (0x05, "R", &u64::MAX.to_le_bytes()), (0x99, "X", &[])]);
        let unknown_kdf = kdbx4(&[(0x42, "$UUID", &[0; 16]), (0x05, "R", &u64::MAX.to_le_bytes())]);
        for file in [mistyped, unknown_type, unknown_kdf] {
            assert_eq!(check_header(&file), Ok(()));
        }
        let mut endless = argon2(4 << 40, 2, 2);
        endless[34..38].copy_from_slice(&u32::MAX.to_le_bytes()); // the KDF parameters' length
        assert_eq!(check_header(&endless), Ok(()));
        assert_eq!(check_header(b"not a database"), Ok(()));
        // A name given twice counts with its last value, as in keepass-rs.
        let twice = |first: u64, last: u64| kdbx4(&[(0x42, "$UUID", &AES_KDBX4), (0x05, "R", &first.to_le_bytes()), (0x05, "R", &last.to_le_bytes())]);
        assert_eq!(check_header(&twice(u64::MAX, 60_000)), Ok(()));
        assert!(check_header(&twice(60_000, u64::MAX)).is_err());
    }
}
