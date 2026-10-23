//! TOTP codes from an entry's attributes, in the layouts other KeePass clients
//! write (#278): an `otpauth://totp/…` URI as KeePassXC and `sic2kdbx` write
//! it, a bare base32 secret or a KeeOtp string in `otp`; KeeTrayTOTP's
//! `TOTP Seed` and `TOTP Settings`; KeePass 2's own `TimeOtp-*` attributes.
//! The usual defaults (6 digits, 30 seconds, SHA-1) apply where a layout
//! leaves them out. Steam's codes are five letters instead of digits.

use crate::edit::value_of;
use base64::Engine;
use keepass::db::{fields, Value};
use serde::Serialize;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use totp_lite::{totp_custom, Sha1, Sha256, Sha512};
use url::Url;
use zeroize::Zeroizing;

/// KeeTrayTOTP's secret (base32) and its `period;digits` (`S` for Steam).
const SEED: &str = "TOTP Seed";
const SETTINGS: &str = "TOTP Settings";
/// What KeePass 2's attributes start with; the `-Secret` ones hold the secret.
const KEEPASS: &str = "TimeOtp-";
const KEEPASS_SECRET: &str = "TimeOtp-Secret";
const KEEPASS_HEX: &str = "TimeOtp-Secret-Hex";
const KEEPASS_BASE32: &str = "TimeOtp-Secret-Base32";
const KEEPASS_BASE64: &str = "TimeOtp-Secret-Base64";
const KEEPASS_LENGTH: &str = "TimeOtp-Length";
const KEEPASS_PERIOD: &str = "TimeOtp-Period";
const KEEPASS_ALGORITHM: &str = "TimeOtp-Algorithm";
/// What `TOTP Settings` has for the length of a Steam code.
const KEETRAY_STEAM: &str = "S";
/// The letters Steam writes its codes with.
const STEAM: &[u8] = b"23456789BCDFGHJKMNPQRTVWXY";

#[derive(Debug, Clone, Copy, PartialEq)]
enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl Algorithm {
    fn name(self) -> &'static str {
        match self {
            Algorithm::Sha1 => "SHA1",
            Algorithm::Sha256 => "SHA256",
            Algorithm::Sha512 => "SHA512",
        }
    }

    /// `SHA1`, `sha-256`, KeePass's `HMAC-SHA-512`, KeeOtp's `Sha256`.
    fn parse(value: &str) -> Result<Algorithm, String> {
        let name = value.to_ascii_uppercase().replace('-', "");
        match name.strip_prefix("HMAC").unwrap_or(&name) {
            "SHA1" => Ok(Algorithm::Sha1),
            "SHA256" => Ok(Algorithm::Sha256),
            "SHA512" => Ok(Algorithm::Sha512),
            _ => Err(format!("Unsupported algorithm {value}")),
        }
    }
}

#[derive(Debug)]
pub struct Totp {
    secret: Zeroizing<Vec<u8>>,
    digits: u32,
    period: u64,
    algorithm: Algorithm,
    /// Steam's five letters (HMAC-SHA-1, `digits` 5).
    steam: bool,
}

/// The current code and how long it stays valid.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Code {
    pub code: String,
    pub remaining: u64,
    pub period: u64,
}

impl Totp {
    fn new(secret: Zeroizing<Vec<u8>>) -> Totp {
        Totp { secret, digits: 6, period: 30, algorithm: Algorithm::Sha1, steam: false }
    }

    fn make_steam(&mut self) {
        (self.steam, self.digits, self.algorithm) = (true, 5, Algorithm::Sha1);
    }

    /// Reads what `otp` holds: an `otpauth://totp/` URI, a KeeOtp string
    /// (`key=…&step=…&size=…&otpHashMode=…`), or a bare base32 secret.
    pub fn parse(value: &str) -> Result<Totp, String> {
        let value = value.trim();
        if is_uri(value) {
            return Totp::uri(value);
        }
        if is_keeotp(value) {
            return Totp::keeotp(value);
        }
        Ok(Totp::new(decode_secret(value)?))
    }

    fn uri(value: &str) -> Result<Totp, String> {
        let url = Url::parse(value).map_err(|e| format!("Not a valid otpauth URI: {e}"))?;
        if url.host_str() != Some("totp") {
            return Err("Only time-based codes (otpauth://totp/) are supported".into());
        }
        let mut totp = Totp::new(Zeroizing::new(Vec::new()));
        let (mut secret, mut length, mut steam) = (None, None, false);
        for (key, value) in url.query_pairs() {
            match key.to_ascii_lowercase().as_str() {
                "secret" => secret = Some(Zeroizing::new(value.into_owned())),
                "digits" => length = Some(value.into_owned()),
                "period" => totp.period = period(&value)?,
                "algorithm" => totp.algorithm = Algorithm::parse(&value)?,
                // KeePassXC's way of marking a Steam secret.
                "encoder" => steam = value.eq_ignore_ascii_case("steam"),
                _ => {}
            }
        }
        totp.secret = decode_secret(&secret.ok_or("The URI has no secret")?)?;
        if steam {
            totp.make_steam();
        } else if let Some(length) = length {
            totp.digits = digits(&length)?;
        }
        Ok(totp)
    }

    /// KeeOtp's query string, as KeePassXC and Keepass2Android read it.
    fn keeotp(value: &str) -> Result<Totp, String> {
        let mut totp = Totp::new(Zeroizing::new(Vec::new()));
        let mut key = None;
        for (name, value) in url::form_urlencoded::parse(value.as_bytes()) {
            match name.to_ascii_lowercase().as_str() {
                "key" => key = Some(Zeroizing::new(value.into_owned())),
                "size" => totp.digits = digits(&value)?,
                "step" => totp.period = period(&value)?,
                "otphashmode" => totp.algorithm = Algorithm::parse(&value)?,
                "type" if !value.eq_ignore_ascii_case("totp") => return Err("Only time-based codes are supported".into()),
                _ => {}
            }
        }
        totp.secret = decode_secret(&key.ok_or("The KeeOtp value has no key")?)?;
        Ok(totp)
    }

    /// KeeTrayTOTP's seed with its settings: `period;digits` (`S` for Steam),
    /// perhaps with a time-correction URL after them.
    fn keetray(seed: &str, settings: &str) -> Result<Totp, String> {
        let mut totp = Totp::new(decode_secret(seed)?);
        let mut parts = settings.split(';').map(str::trim);
        totp.period = period(parts.next().unwrap_or(""))?;
        match parts.next().ok_or("TOTP Settings has no length")? {
            length if length.eq_ignore_ascii_case(KEETRAY_STEAM) => totp.make_steam(),
            length => totp.digits = digits(length)?,
        }
        Ok(totp)
    }

    /// KeePass 2's attributes, the secret in any of its four encodings;
    /// `None` when there is no secret among them.
    fn keepass<'a>(text: &dyn Fn(&str) -> Option<&'a str>) -> Option<Result<Totp, String>> {
        let secret = [
            (KEEPASS_SECRET, Encoding::Utf8),
            (KEEPASS_HEX, Encoding::Hex),
            (KEEPASS_BASE32, Encoding::Base32),
            (KEEPASS_BASE64, Encoding::Base64),
        ]
        .into_iter()
        .find_map(|(name, encoding)| text(name).map(|value| encoding.decode(value)))?;
        let read = || {
            let mut totp = Totp::new(secret?);
            if let Some(length) = text(KEEPASS_LENGTH) {
                totp.digits = digits(length)?;
            }
            if let Some(seconds) = text(KEEPASS_PERIOD) {
                totp.period = period(seconds)?;
            }
            if let Some(name) = text(KEEPASS_ALGORITHM) {
                totp.algorithm = Algorithm::parse(name)?;
            }
            Ok(totp)
        };
        Some(read())
    }

    pub fn code_at(&self, unix_time: u64) -> Code {
        let code = if self.steam {
            steam_code(&self.secret, self.period, unix_time)
        } else {
            match self.algorithm {
                Algorithm::Sha1 => totp_custom::<Sha1>(self.period, self.digits, &self.secret, unix_time),
                Algorithm::Sha256 => totp_custom::<Sha256>(self.period, self.digits, &self.secret, unix_time),
                Algorithm::Sha512 => totp_custom::<Sha512>(self.period, self.digits, &self.secret, unix_time),
            }
        };
        Code { code, remaining: self.period - unix_time % self.period, period: self.period }
    }

    pub fn code_now(&self) -> Code {
        self.code_at(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
    }

    pub fn is_steam(&self) -> bool {
        self.steam
    }

    pub fn period(&self) -> u64 {
        self.period
    }

    pub fn digits(&self) -> u32 {
        self.digits
    }

    /// `SHA1`, `SHA256` or `SHA512`.
    pub fn algorithm(&self) -> &'static str {
        self.algorithm.name()
    }

    /// The secret in base32, unpadded.
    pub fn base32(&self) -> Zeroizing<String> {
        Zeroizing::new(base32::encode(base32::Alphabet::Rfc4648 { padding: false }, &self.secret))
    }

    /// An `otpauth://totp/` URI without a label, the defaults left out.
    pub fn to_uri(&self) -> Zeroizing<String> {
        let mut uri = Zeroizing::new(format!("otpauth://totp/?secret={}", self.base32().as_str()));
        if self.period != 30 {
            uri.push_str(&format!("&period={}", self.period));
        }
        if self.steam {
            uri.push_str("&encoder=steam");
            return uri;
        }
        if self.digits != 6 {
            uri.push_str(&format!("&digits={}", self.digits));
        }
        if self.algorithm != Algorithm::Sha1 {
            uri.push_str(&format!("&algorithm={}", self.algorithm.name()));
        }
        uri
    }
}

/// Steam's code: the same truncated HMAC-SHA-1 value as a TOTP's, written as
/// five of Steam's letters instead of decimal digits. Ten digits from
/// `totp-lite` are that whole value, which is below 2^31.
fn steam_code(secret: &[u8], period: u64, unix_time: u64) -> String {
    let mut value: u64 = totp_custom::<Sha1>(period, 10, secret, unix_time).parse().expect("ten decimal digits");
    let base = STEAM.len() as u64;
    (0..5)
        .map(|_| {
            let letter = STEAM[(value % base) as usize] as char;
            value /= base;
            letter
        })
        .collect()
}

fn digits(value: &str) -> Result<u32, String> {
    value.trim().parse().ok().filter(|d| (6..=10).contains(d)).ok_or_else(|| "Bad digits".into())
}

fn period(value: &str) -> Result<u64, String> {
    value.trim().parse().ok().filter(|p| *p > 0).ok_or_else(|| "Bad period".into())
}

/// Whether `value` is an `otpauth:` URI (any case), not a bare secret.
pub fn is_uri(value: &str) -> bool {
    value.trim_start().get(..8).is_some_and(|scheme| scheme.eq_ignore_ascii_case("otpauth:"))
}

/// An `otpauth://hotp/` URI with a secret that reads as base32.
pub(crate) fn is_hotp(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else { return false };
    url.scheme().eq_ignore_ascii_case("otpauth")
        && url.host_str().is_some_and(|host| host.eq_ignore_ascii_case("hotp"))
        && url.query_pairs().find(|(key, _)| key.eq_ignore_ascii_case("secret")).is_some_and(|(_, secret)| decode_secret(&secret).is_ok())
}

fn is_keeotp(value: &str) -> bool {
    url::form_urlencoded::parse(value.as_bytes()).any(|(name, _)| name.eq_ignore_ascii_case("key"))
}

/// How KeePass 2 may encode the secret.
#[derive(Clone, Copy)]
enum Encoding {
    Utf8,
    Hex,
    Base32,
    Base64,
}

impl Encoding {
    fn decode(self, value: &str) -> Result<Zeroizing<Vec<u8>>, String> {
        let bytes = match self {
            Encoding::Utf8 => Some(value.as_bytes().to_vec()),
            Encoding::Hex => decode_hex(value),
            Encoding::Base32 => return decode_secret(value),
            Encoding::Base64 => base64::engine::general_purpose::STANDARD.decode(value.trim()).ok(),
        };
        bytes.filter(|b| !b.is_empty()).map(Zeroizing::new).ok_or_else(|| "The TOTP secret cannot be read".into())
    }
}

pub(crate) fn decode_hex(value: &str) -> Option<Vec<u8>> {
    let clean: Zeroizing<Vec<u8>> = Zeroizing::new(value.bytes().filter(|b| !b.is_ascii_whitespace()).collect());
    if !clean.len().is_multiple_of(2) {
        return None;
    }
    clean.chunks(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()).collect()
}

/// Base32 as people paste it: any case, with spaces or dashes, padded or not.
fn decode_secret(secret: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    let clean: Zeroizing<String> = Zeroizing::new(
        secret.chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '=').collect::<String>().to_ascii_uppercase(),
    );
    match base32::decode(base32::Alphabet::Rfc4648 { padding: false }, &clean) {
        Some(bytes) if !bytes.is_empty() => Ok(Zeroizing::new(bytes)),
        _ => Err("The TOTP secret is not valid base32".into()),
    }
}

/// What the editor stores in `otp`: a URI or a KeeOtp string kept as typed
/// (once it is known to work), or a bare secret turned into a URI labelled
/// with the entry's title.
pub fn normalize(input: &str, title: &str) -> Result<String, String> {
    let input = input.trim();
    Totp::parse(input)?;
    if is_uri(input) || is_keeotp(input) {
        return Ok(input.to_string());
    }
    let secret: String = input.chars().filter(|c| !c.is_whitespace() && *c != '-').collect::<String>().to_ascii_uppercase();
    let mut url = Url::parse("otpauth://totp/").expect("constant URL");
    url.set_path(if title.is_empty() { "PswManager" } else { title });
    url.query_pairs_mut().append_pair("secret", secret.trim_end_matches('='));
    Ok(url.to_string())
}

/// Whether saving takes `value` as an entry's TOTP: one this app reads, or
/// an HOTP URI kept for other clients ([write]).
pub fn is_storable(value: &str) -> bool {
    Totp::parse(value).is_ok() || is_hotp(value.trim())
}

/// Bitwarden's `steam://SECRET` as a URI this app reads; anything else as it is.
pub fn from_steam_scheme(value: &str) -> String {
    let value = value.trim();
    match value.get(..8).filter(|scheme| scheme.eq_ignore_ascii_case("steam://")) {
        Some(_) => format!("otpauth://totp/?secret={}&encoder=steam", value[8..].trim()),
        None => value.to_string(),
    }
}

/// For a file another app reads: a KeeOtp string as a URI, which more apps
/// read; anything else as it is.
pub fn portable(text: &str) -> Zeroizing<String> {
    match Totp::parse(text) {
        Ok(totp) if is_keeotp(text.trim()) => totp.to_uri(),
        _ => Zeroizing::new(text.to_string()),
    }
}

/// Where an entry keeps its TOTP (see [find]).
enum Found<'a> {
    /// What `otp` holds, as it is.
    Otp(&'a str),
    /// Read from KeeTrayTOTP's or KeePass 2's attributes.
    Layout(Totp),
}

/// The entry's TOTP, looked for in KeePassXC's order so both apps show the
/// same code: KeeTrayTOTP's pair, then `otp` (even what this app cannot
/// read), then KeePass 2's attributes.
fn locate(fields: &HashMap<String, Value<String>>) -> Option<Found<'_>> {
    let text = |name: &str| fields.get(name).map(|v| v.get().as_str()).filter(|v| !v.trim().is_empty());
    if let (Some(seed), Some(settings)) = (text(SEED), text(SETTINGS)) {
        if let Ok(totp) = Totp::keetray(seed, settings) {
            return Some(Found::Layout(totp));
        }
    }
    if let Some(otp) = text(fields::OTP) {
        return Some(Found::Otp(otp));
    }
    Totp::keepass(&text)?.ok().map(Found::Layout)
}

/// The entry's TOTP as the editor shows it: what `otp` holds, or another
/// layout's as a URI; `None` when the entry has none.
pub fn find(fields: &HashMap<String, Value<String>>) -> Option<Zeroizing<String>> {
    Some(match locate(fields)? {
        Found::Otp(otp) => Zeroizing::new(otp.to_string()),
        Found::Layout(totp) => totp.to_uri(),
    })
}

/// Whether the entry has a TOTP, without copying its secret (the listing):
/// the 2FA filter, the TOTP row and the copy actions go by it. An HOTP URI
/// (kept for other clients, #298) gives no code here, so it does not count;
/// a value this app cannot read does, so the entry says why it has no code.
pub fn has(fields: &HashMap<String, Value<String>>) -> bool {
    match locate(fields) {
        Some(Found::Otp(otp)) => !is_hotp(otp.trim()),
        found => found.is_some(),
    }
}

/// Whether an attribute belongs to KeeTrayTOTP's or KeePass 2's TOTP layout.
pub fn is_layout_field(name: &str) -> bool {
    name == SEED || name == SETTINGS || name.starts_with(KEEPASS)
}

/// Writes `typed` (checked to work) into the TOTP layouts `fields` has, as
/// KeePassXC does, so the plugin that wrote each keeps working: KeeTrayTOTP's
/// pair and KeePass 2's attributes where they can hold it, and removed where
/// they cannot; `otp` when the entry has it or nothing else holds the TOTP.
/// Empty `typed` removes them all. A secret written in place of another keeps
/// its protection; a new one is protected.
pub fn write(fields: &mut HashMap<String, Value<String>>, typed: &str, title: &str) -> Result<(), String> {
    let typed = typed.trim();
    if typed.is_empty() {
        fields.retain(|name, _| name != fields::OTP && !is_layout_field(name));
        return Ok(());
    }
    if is_hotp(typed) {
        // Kept for the clients that make its codes (an authenticator's
        // import, #282); this app shows none, and only `otp` holds it.
        fields.retain(|name, _| !is_layout_field(name));
        let protected = fields.get(fields::OTP).is_none_or(Value::is_protected);
        fields.insert(fields::OTP.into(), value_of(typed, protected));
        return Ok(());
    }
    let totp = Totp::parse(typed)?;
    let in_keetray = write_keetray(fields, &totp);
    let in_keepass = write_keepass(fields, &totp);
    if fields.contains_key(fields::OTP) || !(in_keetray || in_keepass) {
        let protected = fields.get(fields::OTP).is_none_or(Value::is_protected);
        fields.insert(fields::OTP.into(), value_of(&normalize(typed, title)?, protected));
    }
    Ok(())
}

/// KeeTrayTOTP's pair, where the entry has it; it holds no algorithm but
/// SHA-1. Returns whether it holds the TOTP now.
fn write_keetray(fields: &mut HashMap<String, Value<String>>, totp: &Totp) -> bool {
    if !(fields.contains_key(SEED) && fields.contains_key(SETTINGS)) {
        return false;
    }
    if totp.algorithm != Algorithm::Sha1 {
        fields.remove(SEED);
        fields.remove(SETTINGS);
        return false;
    }
    let length = if totp.steam { KEETRAY_STEAM.to_string() } else { totp.digits.to_string() };
    let seed = value_of(&totp.base32(), fields[SEED].is_protected());
    fields.insert(SEED.into(), seed);
    fields.insert(SETTINGS.into(), Value::unprotected(format!("{};{length}", totp.period)));
    true
}

/// KeePass 2's attributes, where the entry has a secret among them; they
/// hold no Steam secret and at most 8 digits. Returns whether they hold the
/// TOTP now.
fn write_keepass(fields: &mut HashMap<String, Value<String>>, totp: &Totp) -> bool {
    if !fields.keys().any(|name| name.starts_with(KEEPASS_SECRET)) {
        return false;
    }
    let protected = fields.iter().filter(|(name, _)| name.starts_with(KEEPASS_SECRET)).all(|(_, v)| v.is_protected());
    // The optional ones are written where the entry had them or the value is
    // not KeePass's default.
    let had = |name: &str| fields.contains_key(name);
    let settings = [
        (KEEPASS_LENGTH, had(KEEPASS_LENGTH) || totp.digits != 6, totp.digits.to_string()),
        (KEEPASS_PERIOD, had(KEEPASS_PERIOD) || totp.period != 30, totp.period.to_string()),
        (KEEPASS_ALGORITHM, had(KEEPASS_ALGORITHM) || totp.algorithm != Algorithm::Sha1, format!("HMAC-SHA-{}", &totp.algorithm.name()[3..])),
    ];
    fields.retain(|name, _| !name.starts_with(KEEPASS));
    if totp.steam || totp.digits > 8 {
        return false;
    }
    fields.insert(KEEPASS_BASE32.into(), value_of(&totp.base32(), protected));
    for (name, _, text) in settings.into_iter().filter(|(_, wanted, _)| *wanted) {
        fields.insert(name.into(), Value::unprotected(text));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238, appendix B: the SHA-1 secret "12345678901234567890".
    const RFC_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
    /// The secret of KeePassXC's Steam test, with its times and codes.
    const STEAM_SECRET: &str = "63BEDWCQZKTQWPESARIERL5DTTQFCJTK";

    fn attributes(pairs: &[(&str, &str)]) -> HashMap<String, Value<String>> {
        pairs.iter().map(|(name, text)| (name.to_string(), Value::protected(text.to_string()))).collect()
    }

    fn code_of(fields: &HashMap<String, Value<String>>, at: u64) -> String {
        Totp::parse(&find(fields).unwrap()).unwrap().code_at(at).code
    }

    fn names(fields: &HashMap<String, Value<String>>) -> Vec<&str> {
        let mut names: Vec<&str> = fields.keys().map(String::as_str).collect();
        names.sort();
        names
    }

    #[test]
    fn matches_the_rfc_6238_vectors() {
        let totp = Totp::parse(&format!("otpauth://totp/x?secret={RFC_SECRET}&digits=8")).unwrap();
        assert_eq!(totp.code_at(59).code, "94287082");
        assert_eq!(totp.code_at(1111111109).code, "07081804");
        assert_eq!(totp.code_at(20000000000).code, "65353130");
    }

    #[test]
    fn defaults_to_six_digits_and_thirty_seconds() {
        let totp = Totp::parse(&format!("otpauth://totp/Router?secret={RFC_SECRET}")).unwrap();
        assert_eq!(totp.code_at(59), Code { code: "287082".into(), remaining: 1, period: 30 });
    }

    #[test]
    fn accepts_secrets_as_people_type_them() {
        for secret in ["gezd gnbv gy3t qojq gezd gnbv gy3t qojq", "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ====", RFC_SECRET] {
            assert_eq!(Totp::parse(secret).unwrap().code_at(59).code, "287082", "{secret}");
        }
        assert!(Totp::parse("not base32!").is_err());
        assert!(Totp::parse("otpauth://hotp/x?secret=JBSWY3DP").is_err());
        assert!(Totp::parse("otpauth://totp/x").is_err());
    }

    #[test]
    fn steam_codes_are_five_letters() {
        for uri in [
            format!("otpauth://totp/Steam?secret={STEAM_SECRET}&encoder=steam"),
            format!("otpauth://totp/Steam?secret={STEAM_SECRET}&digits=5&encoder=steam"),
        ] {
            let totp = Totp::parse(&uri).unwrap();
            assert_eq!(totp.code_at(1511200518).code, "FR8RV");
            assert_eq!(totp.code_at(1511200714).code, "9P3VP");
        }
        let tray = attributes(&[(SEED, STEAM_SECRET), (SETTINGS, "30;S")]);
        assert_eq!(code_of(&tray, 1511200518), "FR8RV");
    }

    #[test]
    fn reads_keeotp_strings() {
        let totp = Totp::parse(&format!("key={RFC_SECRET}&step=30&size=8&otpHashMode=Sha1")).unwrap();
        assert_eq!(totp.code_at(59).code, "94287082");
        assert_eq!(Totp::parse(&format!("key={RFC_SECRET}")).unwrap().code_at(59).code, "287082");
        assert!(Totp::parse(&format!("key={RFC_SECRET}&type=Hotp&counter=1")).is_err());
        // RFC 6238's SHA-512 vector: its own 64-byte secret.
        let sha512 = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNA";
        let totp = Totp::parse(&format!("key={sha512}&size=8&otpHashMode=Sha512")).unwrap();
        assert_eq!(totp.code_at(59).code, "90693936");
    }

    #[test]
    fn reads_every_layout_in_keepassxcs_order() {
        let hex = "3132333435363738393031323334353637383930";
        let base64 = "MTIzNDU2Nzg5MDEyMzQ1Njc4OTA=";
        for secret in [(KEEPASS_SECRET, "12345678901234567890"), (KEEPASS_HEX, hex), (KEEPASS_BASE32, RFC_SECRET), (KEEPASS_BASE64, base64)] {
            let keepass = attributes(&[secret, (KEEPASS_LENGTH, "8"), (KEEPASS_ALGORITHM, "HMAC-SHA-1")]);
            assert_eq!(code_of(&keepass, 59), "94287082", "{}", secret.0);
        }
        // RFC 6238's SHA-256 vector: its own 32-byte secret.
        let sha256 = attributes(&[(KEEPASS_SECRET, "12345678901234567890123456789012"), (KEEPASS_LENGTH, "8"), (KEEPASS_ALGORITHM, "HMAC-SHA-256")]);
        assert_eq!(code_of(&sha256, 59), "46119246");
        let other = "JBSWY3DPEHPK3PXP";
        // KeeTrayTOTP's pair comes before `otp`, which comes before KeePass 2's.
        let all = attributes(&[(SEED, RFC_SECRET), (SETTINGS, "30;8;https://time.example.com"), (fields::OTP, other), (KEEPASS_BASE32, other)]);
        assert_eq!(code_of(&all, 59), "94287082");
        let otp_first = attributes(&[(fields::OTP, &format!("otpauth://totp/x?secret={RFC_SECRET}")), (KEEPASS_BASE32, other)]);
        assert_eq!(code_of(&otp_first, 59), "287082");
        // An unreadable pair is passed over; an unreadable `otp` is shown as it is.
        let bad_tray = attributes(&[(SEED, "!"), (SETTINGS, "30;6"), (KEEPASS_BASE32, RFC_SECRET)]);
        assert_eq!(code_of(&bad_tray, 59), "287082");
        assert_eq!(find(&attributes(&[(fields::OTP, "nope!")])).unwrap().as_str(), "nope!");
        assert!(find(&attributes(&[(fields::OTP, " "), ("HmacOtp-Secret", "x")])).is_none());
    }

    #[test]
    fn a_found_layout_shows_as_a_uri() {
        let tray = attributes(&[(SEED, "jbsw y3dp"), (SETTINGS, "60;8")]);
        assert_eq!(find(&tray).unwrap().as_str(), "otpauth://totp/?secret=JBSWY3DP&period=60&digits=8");
        let keepass = attributes(&[(KEEPASS_BASE32, "JBSWY3DP"), (KEEPASS_ALGORITHM, "HMAC-SHA-256")]);
        assert_eq!(find(&keepass).unwrap().as_str(), "otpauth://totp/?secret=JBSWY3DP&algorithm=SHA256");
        let steam = attributes(&[(SEED, "JBSWY3DP"), (SETTINGS, "30;S")]);
        assert_eq!(find(&steam).unwrap().as_str(), "otpauth://totp/?secret=JBSWY3DP&encoder=steam");
    }

    #[test]
    fn a_bare_secret_becomes_a_uri() {
        assert_eq!(normalize(" jbsw y3dp ", "My Bank").unwrap(), "otpauth://totp/My%20Bank?secret=JBSWY3DP");
        for kept in ["otpauth://totp/x?secret=JBSWY3DP&digits=8", "key=JBSWY3DP&size=8"] {
            assert_eq!(normalize(kept, "ignored").unwrap(), kept);
        }
        assert!(normalize("nope!", "x").is_err());
        assert_eq!(portable("key=JBSWY3DP&size=8").as_str(), "otpauth://totp/?secret=JBSWY3DP&digits=8");
        assert_eq!(portable("JBSWY3DP").as_str(), "JBSWY3DP");
    }

    #[test]
    fn writes_back_into_the_layouts_the_entry_has() {
        let text = |f: &HashMap<String, Value<String>>, name: &str| f.get(name).map(|v| v.get().clone());
        // KeeTrayTOTP and KeePass 2 hold it; no `otp` is added beside them.
        let mut fields = attributes(&[(SEED, "JBSWY3DP"), (SETTINGS, "30;6"), (KEEPASS_SECRET, "old"), (KEEPASS_PERIOD, "30")]);
        write(&mut fields, "otpauth://totp/x?secret=GEZDGNBV&period=60&digits=8", "Bank").unwrap();
        assert_eq!(text(&fields, SEED).as_deref(), Some("GEZDGNBV"));
        assert_eq!(text(&fields, SETTINGS).as_deref(), Some("60;8"));
        assert_eq!(names(&fields), [SEED, SETTINGS, KEEPASS_LENGTH, KEEPASS_PERIOD, KEEPASS_BASE32]);
        assert_eq!(text(&fields, KEEPASS_BASE32).as_deref(), Some("GEZDGNBV"));
        assert_eq!((text(&fields, KEEPASS_PERIOD).as_deref(), text(&fields, KEEPASS_LENGTH).as_deref()), (Some("60"), Some("8")));
        assert!(fields[SEED].is_protected() && !fields[SETTINGS].is_protected() && fields[KEEPASS_BASE32].is_protected());

        // A layout that cannot hold it goes, and `otp` takes the TOTP.
        let mut fields = attributes(&[(SEED, "JBSWY3DP"), (SETTINGS, "30;6"), (KEEPASS_BASE32, "JBSWY3DP"), (KEEPASS_LENGTH, "6")]);
        write(&mut fields, "otpauth://totp/x?secret=GEZDGNBV&algorithm=SHA256&digits=10", "Bank").unwrap();
        assert_eq!(names(&fields), [fields::OTP]);
        let mut fields = attributes(&[(KEEPASS_BASE32, "JBSWY3DP")]);
        write(&mut fields, &format!("otpauth://totp/s?secret={STEAM_SECRET}&encoder=steam"), "Steam").unwrap();
        assert_eq!(names(&fields), [fields::OTP]);

        // An entry with `otp` keeps it beside the others; a bare secret becomes a URI.
        let mut fields = attributes(&[(SEED, "JBSWY3DP"), (SETTINGS, "30;6"), (fields::OTP, "JBSWY3DP")]);
        write(&mut fields, "GEZDGNBV", "Bank").unwrap();
        assert_eq!(text(&fields, fields::OTP).as_deref(), Some("otpauth://totp/Bank?secret=GEZDGNBV"));
        assert_eq!(text(&fields, SEED).as_deref(), Some("GEZDGNBV"));

        // Cleared, they all go; HOTP's attributes are not TOTP's.
        let mut fields = attributes(&[(SEED, "A"), (SETTINGS, "30;6"), (fields::OTP, "x"), (KEEPASS_BASE32, "A"), ("HmacOtp-Secret", "h")]);
        write(&mut fields, " ", "Bank").unwrap();
        assert_eq!(names(&fields), ["HmacOtp-Secret"]);
        assert!(write(&mut attributes(&[]), "nope!", "x").is_err());

        // HOTP: kept in `otp` as it is, the other layouts gone.
        let hotp = "otpauth://hotp/bob?secret=GEZDGNBV&counter=4";
        let mut fields = attributes(&[(KEEPASS_BASE32, "JBSWY3DP")]);
        write(&mut fields, hotp, "Bank").unwrap();
        assert_eq!(names(&fields), [fields::OTP]);
        assert_eq!(text(&fields, fields::OTP).as_deref(), Some(hotp));
        assert!(write(&mut attributes(&[]), "otpauth://hotp/bob?secret=!", "x").is_err());
        // Kept, but no TOTP as far as the listing goes.
        assert!(!has(&attributes(&[(fields::OTP, hotp)])));
        assert!(has(&attributes(&[(fields::OTP, "nope!")])));
    }
}
