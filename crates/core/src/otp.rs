//! TOTP codes from an entry's `otp` attribute: an `otpauth://totp/…` URI as
//! KeePassXC and `sic2kdbx` write it, with the usual defaults (6 digits, 30
//! seconds, SHA-1) when the URI leaves them out.

use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use totp_lite::{totp_custom, Sha1, Sha256, Sha512};
use url::Url;
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

pub struct Totp {
    secret: Zeroizing<Vec<u8>>,
    digits: u32,
    period: u64,
    algorithm: Algorithm,
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
    /// Reads an `otpauth://totp/` URI, or a bare base32 secret.
    pub fn parse(value: &str) -> Result<Totp, String> {
        let value = value.trim();
        if !value.to_ascii_lowercase().starts_with("otpauth:") {
            return Ok(Totp { secret: decode_secret(value)?, digits: 6, period: 30, algorithm: Algorithm::Sha1 });
        }
        let url = Url::parse(value).map_err(|e| format!("Not a valid otpauth URI: {e}"))?;
        if url.host_str() != Some("totp") {
            return Err("Only time-based codes (otpauth://totp/) are supported".into());
        }
        let mut totp = Totp { secret: Zeroizing::new(Vec::new()), digits: 6, period: 30, algorithm: Algorithm::Sha1 };
        let mut secret = None;
        for (key, value) in url.query_pairs() {
            match key.to_ascii_lowercase().as_str() {
                "secret" => secret = Some(Zeroizing::new(value.into_owned())),
                "digits" => totp.digits = value.parse().ok().filter(|d| (6..=10).contains(d)).ok_or("Bad digits")?,
                "period" => totp.period = value.parse().ok().filter(|p| *p > 0).ok_or("Bad period")?,
                "algorithm" => {
                    totp.algorithm = match value.to_ascii_uppercase().as_str() {
                        "SHA1" => Algorithm::Sha1,
                        "SHA256" => Algorithm::Sha256,
                        "SHA512" => Algorithm::Sha512,
                        _ => return Err(format!("Unsupported algorithm {value}")),
                    }
                }
                _ => {}
            }
        }
        totp.secret = decode_secret(&secret.ok_or("The URI has no secret")?)?;
        Ok(totp)
    }

    pub fn code_at(&self, unix_time: u64) -> Code {
        let code = match self.algorithm {
            Algorithm::Sha1 => totp_custom::<Sha1>(self.period, self.digits, &self.secret, unix_time),
            Algorithm::Sha256 => totp_custom::<Sha256>(self.period, self.digits, &self.secret, unix_time),
            Algorithm::Sha512 => totp_custom::<Sha512>(self.period, self.digits, &self.secret, unix_time),
        };
        Code { code, remaining: self.period - unix_time % self.period, period: self.period }
    }

    pub fn code_now(&self) -> Code {
        self.code_at(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
    }
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

/// What the editor stores in `otp`: a URI kept as typed (once it is known to
/// work), or a bare secret turned into a URI labelled with the entry's title.
pub fn normalize(input: &str, title: &str) -> Result<String, String> {
    let input = input.trim();
    Totp::parse(input)?;
    if input.to_ascii_lowercase().starts_with("otpauth:") {
        return Ok(input.to_string());
    }
    let secret: String = input.chars().filter(|c| !c.is_whitespace() && *c != '-').collect::<String>().to_ascii_uppercase();
    let mut url = Url::parse("otpauth://totp/").expect("constant URL");
    url.set_path(if title.is_empty() { "PswManager" } else { title });
    url.query_pairs_mut().append_pair("secret", secret.trim_end_matches('='));
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238, appendix B: the SHA-1 secret "12345678901234567890".
    const RFC_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

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
    fn a_bare_secret_becomes_a_uri() {
        assert_eq!(normalize(" jbsw y3dp ", "My Bank").unwrap(), "otpauth://totp/My%20Bank?secret=JBSWY3DP");
        let uri = "otpauth://totp/x?secret=JBSWY3DP&digits=8";
        assert_eq!(normalize(uri, "ignored").unwrap(), uri);
        assert!(normalize("nope!", "x").is_err());
    }
}
