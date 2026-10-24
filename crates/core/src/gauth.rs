//! Importing Google Authenticator's export (#281): its *Transfer accounts*
//! shows QR codes `otpauth-migration://offline?data=…`, each the base64 of a
//! protobuf `MigrationPayload` with some of the accounts; a large export is
//! split over several codes (`batch_size`, `batch_index`, `batch_id`).
//! Google publishes no schema; the field numbers are the ones the community
//! reconstructed (GNOME Authenticator, among others), read here by a small
//! protobuf reader of our own. The codes are collected ([Collected]) until
//! all are in, or the user imports what is there; the accounts become
//! entries through the same [Import] as the file imports.

use crate::cxf::{self, Fields, Import, Skipped};
use crate::edit::EntryData;
use base64::Engine;
use std::collections::BTreeMap;
use std::sync::Mutex;
use zeroize::Zeroizing;

const SCHEME: &str = "otpauth-migration:";
/// The most codes an export is taken to have: Google Authenticator puts about
/// ten accounts in a code, so this is far beyond any real one, and a damaged
/// or made-up code cannot ask for billions.
const MAX_CODES: usize = 255;

/// One account of the export.
struct Account {
    secret: Zeroizing<Vec<u8>>,
    name: String,
    issuer: String,
    /// 1 SHA-1, 2 SHA-256, 3 SHA-512, 4 MD5; 0 unspecified (SHA-1).
    algorithm: u64,
    /// 1 six, 2 eight; 0 unspecified (six).
    digits: u64,
    /// 1 HOTP, 2 TOTP; 0 unspecified (TOTP).
    kind: u64,
    counter: u64,
}

/// One code's payload.
struct Payload {
    accounts: Vec<Account>,
    size: usize,
    index: usize,
    id: u64,
}

/// Whether a QR code's text is Google Authenticator's export.
pub fn is_export(text: &str) -> bool {
    text.trim_start().get(..SCHEME.len()).is_some_and(|start| start.eq_ignore_ascii_case(SCHEME))
}

/// The export's codes read so far: one export's batches, by their index.
#[derive(Default)]
pub struct Collected {
    /// The export's id and its number of codes.
    export: Option<(u64, usize)>,
    batches: BTreeMap<usize, Vec<Account>>,
}

/// The codes read while the app's import page (or sheet) is open, held in
/// memory only: `None` once it is left, the codes imported or the database
/// locked ([Session::cancel]), so a read that ends later keeps nothing.
#[derive(Default)]
pub struct Session(Mutex<Option<Collected>>);

impl Session {
    /// The page opens: no codes read yet.
    pub fn start(&self) {
        *self.0.lock().unwrap() = Some(Collected::default());
    }

    /// Lets go of the codes read so far.
    pub fn cancel(&self) {
        self.0.lock().unwrap().take();
    }

    /// Whether the page is open and has read the code `text` already.
    pub fn is_read(&self, text: &str) -> bool {
        self.0.lock().unwrap().as_ref().is_some_and(|read| read.is_read(text))
    }

    /// Adds a code read ([Collected::add]); `None` when the page is gone.
    pub fn add(&self, text: &str) -> Result<Option<Progress>, String> {
        self.0.lock().unwrap().as_mut().map(|read| read.add(text)).transpose()
    }

    /// The accounts read as entries ([Collected::import]), for `add` to keep;
    /// the codes go once it has, so a save that fails does not cost the user
    /// reading them all again. Held meanwhile: a second import, or a code
    /// read, waits for it.
    pub fn import<R>(&self, add: impl FnOnce(Import) -> Result<R, String>) -> Result<R, String> {
        let mut read = self.0.lock().unwrap();
        let added = add(read.as_ref().ok_or("No codes were read")?.import())?;
        *read = None;
        Ok(added)
    }
}

/// How far the collecting is, for the page to show.
#[derive(Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// The export's codes (1 for a small export).
    pub codes: usize,
    /// Which of them were read, counted from 1.
    pub read: Vec<usize>,
    /// The accounts they hold.
    pub accounts: usize,
}

impl Collected {
    /// Adds the code whose text is `text`. A code of another export than
    /// the ones read before starts over with it; one read again changes
    /// nothing.
    pub fn add(&mut self, text: &str) -> Result<Progress, String> {
        let payload = parse(text)?;
        if self.export != Some((payload.id, payload.size)) {
            *self = Collected { export: Some((payload.id, payload.size)), batches: BTreeMap::new() };
        }
        self.batches.entry(payload.index).or_insert(payload.accounts);
        Ok(self.progress())
    }

    /// Whether the code whose text is `text` was read already (the
    /// clipboard may still hold the last snip).
    pub fn is_read(&self, text: &str) -> bool {
        parse(text).is_ok_and(|payload| self.export.is_some_and(|(id, _)| id == payload.id) && self.batches.contains_key(&payload.index))
    }

    /// Which codes are read, and the accounts they hold.
    fn size(&self) -> usize {
        self.export.map_or(0, |(_, size)| size)
    }

    pub fn progress(&self) -> Progress {
        Progress {
            codes: self.size(),
            read: self.batches.keys().map(|index| index + 1).collect(),
            accounts: self.batches.values().map(Vec::len).sum(),
        }
    }

    /// The accounts read so far as entries; the codes not read are named
    /// among what did not come over.
    pub fn import(&self) -> Import {
        let mut import = Import { exporter: "Google Authenticator".into(), entries: Vec::new(), skipped: Vec::new() };
        for account in self.batches.values().flatten() {
            let (title, user) = labels(account);
            match uri(account, &title, &user) {
                Ok(otp) => {
                    let mut data = EntryData::default();
                    data.title = title;
                    data.username = user;
                    data.otp = otp;
                    cxf::finish(&mut import, data, Fields::default(), Vec::new(), Vec::new(), (None, None));
                }
                Err(why) => import.skipped.push(Skipped { title, why }),
            }
        }
        let missing: Vec<String> = (0..self.size()).filter(|i| !self.batches.contains_key(i)).map(|i| (i + 1).to_string()).collect();
        if !missing.is_empty() {
            let why = format!("Not read: the export's code {} of {}; import again with it", missing.join(", "), self.size());
            import.skipped.push(Skipped { title: "Google Authenticator".into(), why });
        }
        import
    }
}

/// The entry's title and user name: the issuer and the name, or the two
/// halves of an `Issuer:user` name when the issuer is not given.
fn labels(account: &Account) -> (String, String) {
    let name = account.name.trim();
    let issuer = account.issuer.trim();
    match name.split_once(':') {
        Some((label, user)) if issuer.is_empty() || issuer == label.trim() => (label.trim().to_string(), user.trim().to_string()),
        _ if issuer.is_empty() => (name.to_string(), String::new()),
        _ => (issuer.to_string(), name.to_string()),
    }
}

/// The account as an `otpauth://` URI: TOTP as `totp`, HOTP as `hotp` (kept
/// for other clients; this app shows no code for it). The export has no
/// period: Google Authenticator's codes are 30 seconds.
fn uri(account: &Account, title: &str, user: &str) -> Result<String, String> {
    if account.secret.is_empty() {
        return Err("An account without its secret".into());
    }
    let algorithm = match account.algorithm {
        0 | 1 => "SHA1",
        2 => "SHA256",
        3 => "SHA512",
        _ => return Err("An MD5 account: PswManager cannot make its codes".into()),
    };
    let digits = if account.digits == 2 { "8" } else { "6" };
    let hotp = account.kind == 1;
    let secret = Zeroizing::new(base32::encode(base32::Alphabet::Rfc4648 { padding: false }, &account.secret));
    let mut url = url::Url::parse(if hotp { "otpauth://hotp/" } else { "otpauth://totp/" }).expect("constant URL");
    url.set_path(&match (title.is_empty(), user.is_empty()) {
        (false, false) => format!("{title}:{user}"),
        (false, true) => title.to_string(),
        _ => user.to_string(),
    });
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("secret", &secret);
        if !title.is_empty() {
            query.append_pair("issuer", title);
        }
        query.append_pair("algorithm", algorithm);
        query.append_pair("digits", digits);
        if hotp {
            query.append_pair("counter", &account.counter.to_string());
        }
    }
    Ok(url.to_string())
}

/// A code's text read as its payload.
fn parse(text: &str) -> Result<Payload, String> {
    let not_export = || "Not a code of Google Authenticator's export".to_string();
    if !is_export(text) {
        return Err(not_export());
    }
    let url = url::Url::parse(text.trim()).map_err(|_| not_export())?;
    // Decoded from the query, a `+` of the base64 not written as `%2B` reads as a space.
    let data = url.query_pairs().find(|(key, _)| key == "data").map(|(_, value)| Zeroizing::new(value.replace(' ', "+"))).ok_or_else(not_export)?;
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new().with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let bytes = Zeroizing::new(engine.decode(data.as_bytes()).map_err(|_| "The code's data is damaged".to_string())?);
    payload(&bytes).ok_or_else(|| "The code's data is damaged".into())
}

fn payload(bytes: &[u8]) -> Option<Payload> {
    let mut payload = Payload { accounts: Vec::new(), size: 1, index: 0, id: 0 };
    let mut wire = Wire(bytes);
    while let Some(field) = wire.next_field() {
        match field? {
            (1, Value::Bytes(account)) => payload.accounts.push(account_of(account)?),
            // A negative int32 comes as a huge varint: refused, as too many codes.
            (3, Value::Number(size)) => payload.size = usize::try_from(size).ok().filter(|&size| size <= MAX_CODES)?.max(1),
            (4, Value::Number(index)) => payload.index = usize::try_from(index).ok()?,
            (5, Value::Number(id)) => payload.id = id,
            _ => {}
        }
    }
    (payload.index < payload.size).then_some(payload)
}

fn account_of(bytes: &[u8]) -> Option<Account> {
    let mut account = Account { secret: Zeroizing::new(Vec::new()), name: String::new(), issuer: String::new(), algorithm: 0, digits: 0, kind: 0, counter: 0 };
    let text = |bytes: &[u8]| String::from_utf8(bytes.to_vec()).ok();
    let mut wire = Wire(bytes);
    while let Some(field) = wire.next_field() {
        match field? {
            (1, Value::Bytes(secret)) => account.secret = Zeroizing::new(secret.to_vec()),
            (2, Value::Bytes(name)) => account.name = text(name)?,
            (3, Value::Bytes(issuer)) => account.issuer = text(issuer)?,
            (4, Value::Number(algorithm)) => account.algorithm = algorithm,
            (5, Value::Number(digits)) => account.digits = digits,
            (6, Value::Number(kind)) => account.kind = kind,
            (7, Value::Number(counter)) => account.counter = counter,
            _ => {}
        }
    }
    Some(account)
}

// ---------------------------------------------------------------- protobuf

/// A field's value: a varint, or length-delimited bytes (strings, messages).
enum Value<'a> {
    Number(u64),
    Bytes(&'a [u8]),
}

/// A protobuf message's bytes, read field by field.
struct Wire<'a>(&'a [u8]);

impl<'a> Wire<'a> {
    /// The next field: `None` at the end, `Some(None)` for one that cannot
    /// be read (the caller gives up then).
    fn next_field(&mut self) -> Option<Option<(u64, Value<'a>)>> {
        (!self.0.is_empty()).then(|| self.field())
    }

    fn varint(&mut self) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = self.0.split_first()?;
            self.0 = rest;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn field(&mut self) -> Option<(u64, Value<'a>)> {
        let key = self.varint()?;
        let value = match key & 7 {
            0 => Value::Number(self.varint()?),
            2 => {
                let len = usize::try_from(self.varint()?).ok()?;
                let (bytes, rest) = (self.0.get(..len)?, self.0.get(len..)?);
                self.0 = rest;
                Value::Bytes(bytes)
            }
            // Fixed 64 and 32 bits: no field of the export has them, but skipped rightly.
            1 => {
                self.0 = self.0.get(8..)?;
                Value::Number(0)
            }
            5 => {
                self.0 = self.0.get(4..)?;
                Value::Number(0)
            }
            _ => return None,
        };
        Some((key >> 3, value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otp::Totp;

    fn varint(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    fn number(field: u64, value: u64, out: &mut Vec<u8>) {
        varint(field << 3, out);
        varint(value, out);
    }

    fn bytes(field: u64, value: &[u8], out: &mut Vec<u8>) {
        varint(field << 3 | 2, out);
        varint(value.len() as u64, out);
        out.extend_from_slice(value);
    }

    /// A made-up account: its secret, name and issuer, then its algorithm,
    /// digit count, type and counter.
    fn account(secret: &[u8], name: &str, issuer: &str, numbers: [u64; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        bytes(1, secret, &mut out);
        bytes(2, name.as_bytes(), &mut out);
        bytes(3, issuer.as_bytes(), &mut out);
        for (field, value) in (4..).zip(numbers) {
            number(field, value, &mut out);
        }
        out
    }

    /// One code of an export: its accounts, and batch size, index and id.
    fn code(accounts: &[Vec<u8>], size: u64, index: u64, id: u64) -> String {
        let mut out = Vec::new();
        for a in accounts {
            bytes(1, a, &mut out);
        }
        number(2, 1, &mut out);
        number(3, size, &mut out);
        number(4, index, &mut out);
        number(5, id, &mut out);
        let data = base64::engine::general_purpose::STANDARD.encode(out);
        let mut url = url::Url::parse("otpauth-migration://offline").unwrap();
        url.query_pairs_mut().append_pair("data", &data);
        url.to_string()
    }

    const RFC_SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn reads_an_export_in_two_codes() {
        let first = code(&[account(RFC_SECRET, "alice@example.com", "Example", [2, 2, 2, 0])], 2, 0, 77);
        let second = code(&[account(RFC_SECRET, "Bank:bob", "", [1, 1, 1, 4]), account(RFC_SECRET, "old", "Legacy", [4, 1, 2, 0])], 2, 1, 77);
        let mut collected = Collected::default();
        assert_eq!(collected.add(&second).unwrap(), Progress { codes: 2, read: vec![2], accounts: 2 });
        let import = collected.import();
        assert!(import.skipped.iter().any(|s| s.why.contains("code 1 of 2")));
        assert_eq!(collected.add(&first).unwrap(), Progress { codes: 2, read: vec![1, 2], accounts: 3 });
        assert_eq!(collected.add(&first).unwrap().accounts, 3);

        let import = collected.import();
        let titles: Vec<(&str, &str)> = import.entries.iter().map(|e| (e.data.title.as_str(), e.data.username.as_str())).collect();
        assert_eq!(titles, [("Example", "alice@example.com"), ("Bank", "bob")]);
        let totp = Totp::parse(&import.entries[0].data.otp).unwrap();
        assert_eq!((totp.algorithm(), totp.digits(), totp.code_at(59).code.as_str()), ("SHA256", 8, "32247374"));
        assert!(import.entries[1].data.otp.starts_with("otpauth://hotp/Bank:bob?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"));
        assert!(import.entries[1].data.otp.ends_with("&counter=4"));
        assert_eq!(import.skipped.len(), 1);
        assert!(import.skipped[0].why.contains("MD5"));
    }

    #[test]
    fn a_code_read_already_is_known() {
        let (first, second) = (code(&[], 2, 0, 5), code(&[], 2, 1, 5));
        let mut collected = Collected::default();
        collected.add(&first).unwrap();
        assert!(collected.is_read(&first) && !collected.is_read(&second) && !collected.is_read("x"));
    }

    #[test]
    fn a_session_keeps_codes_only_while_open() {
        let session = Session::default();
        let text = code(&[account(RFC_SECRET, "a", "A", [1, 1, 2, 0])], 1, 0, 4);
        assert_eq!(session.add(&text).unwrap(), None);
        let count = |import: Import| Ok(import.entries.len());
        assert!(session.import(count).is_err());
        session.start();
        assert_eq!(session.add(&text).unwrap().unwrap().accounts, 1);
        assert!(session.is_read(&text));
        // Kept while the entries are not.
        assert!(session.import(|_| Err::<(), _>("The file cannot be saved".into())).is_err());
        assert_eq!(session.import(count).unwrap(), 1);
        assert!(session.import(count).is_err());
        session.start();
        session.cancel();
        assert_eq!(session.add(&text).unwrap(), None);
    }

    #[test]
    fn another_export_starts_over() {
        let mut collected = Collected::default();
        collected.add(&code(&[account(RFC_SECRET, "a", "A", [1, 1, 2, 0])], 2, 0, 1)).unwrap();
        let other = collected.add(&code(&[account(RFC_SECRET, "b", "B", [1, 1, 2, 0])], 1, 0, 2)).unwrap();
        assert_eq!(other, Progress { codes: 1, read: vec![1], accounts: 1 });
    }

    #[test]
    fn says_what_is_wrong_with_a_code() {
        let mut collected = Collected::default();
        assert!(collected.add("otpauth://totp/x?secret=JBSWY3DP").unwrap_err().contains("Not a code"));
        assert!(collected.add("otpauth-migration://offline?data=%%%").unwrap_err().contains("damaged"));
        // A field cut short.
        let cut = base64::engine::general_purpose::STANDARD.encode([0x0a, 0x10, 0x01]);
        assert!(collected.add(&format!("otpauth-migration://offline?data={cut}")).unwrap_err().contains("damaged"));
        // Billions of codes (a negative batch size), or more than an export has.
        for size in [u64::MAX, 256] {
            assert!(collected.add(&code(&[], size, 0, 9)).unwrap_err().contains("damaged"));
        }
        // A `+` of the base64 written as is reads back.
        let plus = (0..=255u8).map(|b| code(&[account(&[0xfb, b, 0xef], "p", "P", [1, 1, 2, 0])], 1, 0, 3)).find(|c| c.contains("%2B")).unwrap();
        assert!(collected.add(&plus.replace("%2B", "+")).is_ok());
    }
}
