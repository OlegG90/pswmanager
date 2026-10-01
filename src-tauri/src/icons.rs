//! Site icons for entries: fetched directly from each site, cached in the data
//! folder under names that do not say the host, never written into the database.

use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use url::Url;
use zeroize::Zeroizing;

/// A site that had no icon is asked again after this long.
const RETRY_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const PAGE_LIMIT: u64 = 512 * 1024;
const ICON_LIMIT: u64 = 256 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);
const PARALLEL_FETCHES: usize = 6;

/// An entry's URL as a web address on a named site. A URL without a scheme
/// (`example.com/login`, as SafeInCloud stores them) counts as https. Bare
/// words ("router", "localhost") and IP addresses are not sites.
pub fn web_url(url: &str) -> Option<Url> {
    let url = url.trim();
    if url.is_empty() || url.chars().any(char::is_whitespace) {
        return None;
    }
    let parsed = match Url::parse(url) {
        Ok(u) => u,
        Err(url::ParseError::RelativeUrlWithoutBase) => Url::parse(&format!("https://{url}")).ok()?,
        Err(_) => return None,
    };
    let is_site = matches!(parsed.host(), Some(url::Host::Domain(d)) if d.trim_end_matches('.').contains('.'));
    (matches!(parsed.scheme(), "http" | "https") && is_site).then_some(parsed)
}

/// The site of an entry's URL, for its icon.
pub fn host_of(url: &str) -> Option<String> {
    let host = web_url(url)?.host_str()?.trim_end_matches('.').to_string();
    is_safe_host(&host).then_some(host)
}

/// True for an https address on a named host: where a site may send the app
/// for its page or icon (its own domain, another domain it moved to, the CDN
/// it keeps its icons on). Never plain http or an IP address, so a page
/// cannot point the app at a device on the local network.
fn fetchable(url: &Url) -> bool {
    url.scheme() == "https" && matches!(url.host(), Some(url::Host::Domain(d)) if d.trim_end_matches('.').contains('.'))
}

/// Hosts become file names, so only plain host characters are allowed.
fn is_safe_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('.')
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

/// The image type of `bytes`, from their signature; `None` for anything that
/// is not an image (an HTML error page served as favicon.ico, for example).
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).trim_start().to_ascii_lowercase();
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0, 0, 1, 0, ..] => Some("image/x-icon"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ if text.starts_with("<svg") || (text.starts_with("<?xml") && text.contains("<svg")) => Some("image/svg+xml"),
        _ => None,
    }
}

/// `bytes` as a `data:` URL for an `<img>`, if they are an image.
pub fn data_url(bytes: &[u8]) -> Option<String> {
    let mime = sniff(bytes)?;
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Candidate icon URLs a site's page declares, best first: icons with a
/// declared size of 32px or more, then Apple touch icons, then the rest.
fn icon_links(html: &str, base: &Url) -> Vec<Url> {
    let mut found: Vec<(u8, Url)> = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut rest = 0;
    while let Some(start) = lower[rest..].find("<link").map(|i| rest + i) {
        let end = lower[start..].find('>').map_or(lower.len(), |i| start + i);
        let tag = &html[start..end];
        rest = end;
        let (Some(rel), Some(href)) = (attr(tag, "rel"), attr(tag, "href")) else { continue };
        let rel = rel.to_ascii_lowercase();
        if !rel.split_whitespace().any(|r| r == "icon" || r == "apple-touch-icon") {
            continue;
        }
        let Ok(url) = base.join(href.trim()) else { continue };
        if !fetchable(&url) {
            continue;
        }
        let big = attr(tag, "sizes").is_some_and(|s| largest_size(&s) >= 32);
        let rank = match (big, rel.contains("apple-touch-icon")) {
            (true, false) => 0,
            (_, true) => 1,
            (false, false) => 2,
        };
        found.push((rank, url));
    }
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, url)| url).collect()
}

/// The value of attribute `name` in a tag, quoted or not.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name).map(|i| from + i) {
        from = i + name.len();
        let before_ok = i > 0 && lower.as_bytes()[i - 1].is_ascii_whitespace();
        let after = lower[from..].trim_start();
        if !before_ok || !after.starts_with('=') {
            continue;
        }
        let value_start = tag.len() - after.len() + 1;
        let value = tag[value_start..].trim_start();
        return Some(match value.chars().next()? {
            q @ ('"' | '\'') => value[1..].split(q).next()?.to_string(),
            _ => value.split(|c: char| c.is_whitespace() || c == '/').next()?.to_string(),
        });
    }
    None
}

/// The largest square size in a `sizes` attribute ("16x16 32x32" → 32).
fn largest_size(sizes: &str) -> u32 {
    sizes
        .split_whitespace()
        .filter_map(|s| s.to_ascii_lowercase().split('x').next()?.parse().ok())
        .max()
        .unwrap_or(0)
}

/// Written into a `.miss` marker. Markers without it come from versions that
/// could not check most sites' certificates, and are tried again at once.
const MISS_MARK: &[u8] = b"2";

/// The Credential Manager entry holding the key the cache's file names are made with.
const KEY_NAME: &str = "icon-cache-key";

/// The icon cache: `<dir>/<name>` holds an icon, `<dir>/<name>.miss` marks a
/// site that had none, where `<name>` is the host hashed with a key only this
/// Windows user has ([Cache::name]): the folder does not say which sites the
/// databases hold. Without the key (the Credential Manager refused it), the
/// icons are kept in memory for the session only.
pub struct Cache {
    dir: PathBuf,
    key: Option<Zeroizing<[u8; 32]>>,
    /// Without a key: each host's icon, or none, for this session.
    memory: Mutex<HashMap<String, Option<Vec<u8>>>>,
}

impl Cache {
    /// The cache in `icons` beside the state file, with this Windows user's
    /// key (made on first use), its folder tidied ([Cache::migrate]).
    pub fn in_data_dir(data_dir: &Path) -> Self {
        let (key, fresh) = cache_key().map_or((None, false), |(key, fresh)| (Some(key), fresh));
        Self::with_key(data_dir, key, fresh)
    }

    fn with_key(data_dir: &Path, key: Option<Zeroizing<[u8; 32]>>, fresh: bool) -> Self {
        let cache = Cache { dir: data_dir.join("icons"), key, memory: Mutex::default() };
        cache.migrate(fresh);
        cache
    }

    /// The marker of a site named `name` that had no icon.
    fn miss(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.miss"))
    }

    /// The file name for `host`: HMAC-SHA256 of it with the key, in hex. A
    /// plain hash would not do: common hosts can be guessed and hashed.
    fn name(&self, host: &str) -> Option<String> {
        let key = self.key.as_ref()?;
        let mut mac = Hmac::<Sha256>::new_from_slice(key.as_slice()).expect("HMAC takes a key of any length");
        mac.update(host.as_bytes());
        Some(crate::dbfile::hex(&mac.finalize().into_bytes()))
    }

    /// Tidies the folder at start: files an earlier version named by host (a
    /// host has a dot, a name made with the key does not) are renamed, or
    /// removed without a key; half-written files are removed; and with a
    /// `fresh` key, so are the files an earlier key named, which nothing finds
    /// any more.
    fn migrate(&self, fresh: bool) {
        let Ok(entries) = fs::read_dir(&self.dir) else { return };
        // Listed first: a file renamed here must not come up again as an earlier key's.
        let files: Vec<(PathBuf, String)> =
            entries.flatten().map(|e| (e.path(), e.file_name().to_string_lossy().into_owned())).collect();
        for (path, file) in files {
            let (host, suffix) = file.strip_suffix(".miss").map_or((file.as_str(), ""), |host| (host, ".miss"));
            if file.ends_with(".tmp") || (!host.contains('.') && fresh) {
                let _ = fs::remove_file(&path);
                continue;
            }
            if !host.contains('.') {
                continue;
            }
            let target = is_safe_host(host).then(|| self.name(host)).flatten().map(|name| self.dir.join(format!("{name}{suffix}")));
            if let Some(to) = target.filter(|to| !to.exists()) {
                if fs::rename(&path, to).is_ok() {
                    continue;
                }
            }
            let _ = fs::remove_file(&path);
        }
    }

    pub fn get(&self, host: &str) -> Option<Vec<u8>> {
        if !is_safe_host(host) {
            return None;
        }
        match self.name(host) {
            Some(name) => fs::read(self.dir.join(name)).ok(),
            None => self.memory.lock().unwrap().get(host).cloned().flatten(),
        }
    }

    /// True when the host has an icon, or was tried recently without one (in
    /// memory, without a key: tried this session).
    fn is_settled(&self, host: &str, now: SystemTime) -> bool {
        let Some(name) = self.name(host) else { return self.memory.lock().unwrap().contains_key(host) };
        if self.dir.join(&name).is_file() {
            return true;
        }
        let marker = self.miss(&name);
        let current = fs::read(&marker).is_ok_and(|mark| mark == MISS_MARK);
        let missed = fs::metadata(&marker).and_then(|m| m.modified());
        current && missed.is_ok_and(|t| now.duration_since(t).unwrap_or_default() < RETRY_AFTER)
    }

    fn put(&self, host: &str, icon: Option<&[u8]>) -> std::io::Result<()> {
        let Some(name) = self.name(host) else {
            self.memory.lock().unwrap().insert(host.to_string(), icon.map(<[u8]>::to_vec));
            return Ok(());
        };
        fs::create_dir_all(&self.dir)?;
        match icon {
            Some(bytes) => {
                let tmp = self.dir.join(format!("{name}.tmp"));
                fs::write(&tmp, bytes)?;
                fs::rename(&tmp, self.dir.join(&name))?;
                let _ = fs::remove_file(self.miss(&name));
                Ok(())
            }
            None => fs::write(self.miss(&name), MISS_MARK),
        }
    }

    /// Fetches the icons of `hosts` that are not settled yet, a few sites at a
    /// time so one slow site does not hold up the rest, calling `ready(host)`
    /// for each icon that arrived. Blocking; run it on a thread.
    pub fn fetch_missing(&self, hosts: Vec<String>, ready: impl Fn(&str) + Sync) {
        let now = SystemTime::now();
        let queue = Mutex::new(hosts.into_iter().filter(|h| is_safe_host(h) && !self.is_settled(h, now)));
        let agent = http_agent(TIMEOUT, true);
        std::thread::scope(|scope| {
            for _ in 0..PARALLEL_FETCHES {
                scope.spawn(|| {
                    while let Some(host) = queue.lock().unwrap().next() {
                        let icon = fetch_icon(&agent, &host);
                        if self.put(&host, icon.as_deref()).is_ok() && icon.is_some() {
                            ready(&host);
                        }
                    }
                });
            }
        });
    }
}

/// This Windows user's key for the cache's file names, from the Credential
/// Manager, and whether it was just made (none there, or not a key): made and
/// kept there then. `None` when it cannot be kept.
fn cache_key() -> Option<(Zeroizing<[u8; 32]>, bool)> {
    if let Some(key) = crate::credentials::read(KEY_NAME).and_then(|stored| parse_key(&stored)) {
        return Some((key, false));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    getrandom::fill(key.as_mut()).ok()?;
    crate::credentials::write(KEY_NAME, &Zeroizing::new(crate::dbfile::hex(key.as_ref()))).ok()?;
    Some((key, true))
}

/// A key as [cache_key] keeps it: 64 hex digits.
fn parse_key(stored: &str) -> Option<Zeroizing<[u8; 32]>> {
    if stored.len() != 64 || !stored.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut key = Zeroizing::new([0u8; 32]);
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&stored[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(key)
}

/// An HTTP client with the OS's TLS (no `ring`, which needs clang on ARM64).
/// With `status_as_error`, an answer that is not 2xx is an error.
pub fn http_agent(timeout: Duration, status_as_error: bool) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(status_as_error)
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                // Windows' own trust store; the default list is not there without webpki-roots.
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into()
}

/// Asks the site itself, over https: the icons its start page declares
/// (wherever the site keeps them), then `/favicon.ico` where the start page
/// ended up (a site may redirect to another domain), then on the host itself.
/// This is what a browser opening the site fetches; only the host is sent —
/// nothing from the entry — and no third-party icon service is asked.
fn fetch_icon(agent: &ureq::Agent, host: &str) -> Option<Vec<u8>> {
    let get = |url: &Url, limit: u64| -> Option<(Url, Vec<u8>)> {
        let mut response = agent.get(url.as_str()).call().ok()?;
        let landed = Url::parse(&ureq::ResponseExt::get_uri(&response).to_string()).ok()?;
        if !fetchable(&landed) {
            return None;
        }
        Some((landed, response.body_mut().with_config().limit(limit).read_to_vec().ok()?))
    };
    let start = Url::parse(&format!("https://{host}/")).ok()?;
    let mut candidates = Vec::new();
    if let Some((landed, html)) = get(&start, PAGE_LIMIT) {
        candidates = icon_links(&String::from_utf8_lossy(&html), &landed);
        candidates.extend(landed.join("/favicon.ico"));
    }
    candidates.extend(start.join("/favicon.ico"));
    candidates.dedup();
    candidates.into_iter().find_map(|url| get(&url, ICON_LIMIT).map(|(_, bytes)| bytes).filter(|b| sniff(b).is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_come_from_web_addresses_only() {
        assert_eq!(host_of("https://Mail.Example.com/login?x=1").as_deref(), Some("mail.example.com"));
        assert_eq!(host_of("example.com").as_deref(), Some("example.com"));
        assert_eq!(host_of("example.com/path").as_deref(), Some("example.com"));
        assert_eq!(host_of("http://user:pw@example.org:8080/").as_deref(), Some("example.org"));
        let not_sites =
            ["", "router", "ftp://example.com", "mailto:me@example.com", "10 0 0 1", "http://localhost/", "192.168.1.1", "https://[::1]/"];
        for not_a_site in not_sites {
            assert_eq!(host_of(not_a_site), None, "{not_a_site}");
        }
    }

    #[test]
    fn web_urls_are_normalised() {
        let url = web_url(" example.com/?next=http://x ").unwrap();
        assert_eq!(url.as_str(), "https://example.com/?next=http://x");
        assert_eq!(web_url("http://example.com").unwrap().as_str(), "http://example.com/");
    }

    #[test]
    fn only_https_on_named_hosts_is_fetched() {
        let ok = |url: &str| fetchable(&Url::parse(url).unwrap());
        assert!(ok("https://www.example.com/i.png"));
        assert!(ok("https://cdn.example.net/i.png"));
        assert!(!ok("http://www.example.com/i.png"));
        assert!(!ok("https://192.168.1.1/i.png"));
        assert!(!ok("https://[::1]/i.png"));
        assert!(!ok("https://router/i.png"));
        assert!(!ok("file:///C:/i.png"));
    }

    #[test]
    fn only_images_are_accepted() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(sniff(&[0, 0, 1, 0, 1, 0]), Some("image/x-icon"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"  <svg xmlns='http://www.w3.org/2000/svg'/>"), Some("image/svg+xml"));
        assert_eq!(sniff(b"<?xml version='1.0'?><svg/>"), Some("image/svg+xml"));
        assert_eq!(sniff(b"<!doctype html><html>Not found</html>"), None);
        assert_eq!(sniff(b""), None);
        assert!(data_url(b"GIF89a....").unwrap().starts_with("data:image/gif;base64,"));
    }

    #[test]
    fn page_icons_are_ranked_and_resolved() {
        let base = Url::parse("https://example.com/app/").unwrap();
        let html = r#"<html><head>
            <link rel="stylesheet" href="/site.css">
            <LINK REL="shortcut icon" HREF="/favicon.ico">
            <link rel=apple-touch-icon href=touch.png>
            <link href='//cdn.example.com/i/64.png' sizes='16x16 64x64' rel='icon'>
            <link rel="icon" href="https://static.example.net/icon.png">
            <link rel="icon" href="http://example.com/plain.ico">
            <link rel="icon" href="https://10.0.0.1/local.ico">
            <link rel="icon" href="javascript:alert(1)">
        </head></html>"#;
        let urls: Vec<String> = icon_links(html, &base).iter().map(Url::to_string).collect();
        assert_eq!(
            urls,
            [
                "https://cdn.example.com/i/64.png",
                "https://example.com/app/touch.png",
                "https://example.com/favicon.ico",
                "https://static.example.net/icon.png",
            ]
        );
    }

    #[test]
    fn attributes_are_matched_whole() {
        assert_eq!(attr(r#"<link data-href="x" href="y">"#, "href").as_deref(), Some("y"));
        assert_eq!(attr("<link rel=icon/>", "rel").as_deref(), Some("icon"));
        assert_eq!(attr("<link rel>", "rel"), None);
    }

    /// Needs the network: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn certificates_are_checked_against_the_windows_store() {
        let agent = http_agent(TIMEOUT, true);
        assert!(agent.get("https://github.com/").call().is_ok());
        assert!(agent.get("https://self-signed.badssl.com/").call().is_err());
    }

    fn key(byte: u8) -> Option<Zeroizing<[u8; 32]>> {
        Some(Zeroizing::new([byte; 32]))
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir.join("icons")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn cache_keeps_icons_and_remembers_misses() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::with_key(dir.path(), key(1), false);
        let now = SystemTime::now();
        assert!(!cache.is_settled("example.com", now));
        cache.put("example.com", None).unwrap();
        assert!(cache.is_settled("example.com", now));
        assert!(!cache.is_settled("example.com", now + RETRY_AFTER + Duration::from_secs(1)));
        // A marker from an older version is tried again at once.
        let name = cache.name("old.example.com").unwrap();
        fs::write(dir.path().join("icons").join(format!("{name}.miss")), b"").unwrap();
        assert!(!cache.is_settled("old.example.com", now));
        cache.put("example.com", Some(b"\x89PNG....")).unwrap();
        assert_eq!(cache.get("example.com").unwrap(), b"\x89PNG....");
        assert!(!cache.miss(&cache.name("example.com").unwrap()).exists());
        assert_eq!(cache.get("../pswm.json"), None);
        // No file says a host.
        assert!(files(dir.path()).iter().all(|f| !f.contains("example")));
    }

    #[test]
    fn file_names_are_the_host_hashed_with_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let (one, other) = (Cache::with_key(dir.path(), key(1), false), Cache::with_key(dir.path(), key(2), false));
        let name = one.name("example.com").unwrap();
        assert_eq!(name.len(), 64);
        assert!(!name.contains('.'));
        assert_eq!(one.name("example.com").unwrap(), name);
        assert_ne!(other.name("example.com").unwrap(), name);
        assert_ne!(one.name("example.org").unwrap(), name);
    }

    #[test]
    fn a_stored_key_is_64_hex_digits() {
        let hex = "00ff".repeat(16);
        assert_eq!(parse_key(&hex).unwrap()[..2], [0x00, 0xff]);
        for bad in ["", "00ff", &"0g".repeat(32), &"+1".repeat(32), &"00".repeat(33)] {
            assert!(parse_key(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn host_named_files_are_renamed_or_removed() {
        let dir = tempfile::tempdir().unwrap();
        let icons = dir.path().join("icons");
        fs::create_dir_all(&icons).unwrap();
        fs::write(icons.join("example.com"), b"\x89PNG....").unwrap();
        fs::write(icons.join("missing.example.org.miss"), MISS_MARK).unwrap();
        fs::write(icons.join("half.example.net.tmp"), b"..").unwrap();

        let cache = Cache::with_key(dir.path(), key(1), false);
        assert_eq!(cache.get("example.com").unwrap(), b"\x89PNG....");
        assert!(cache.is_settled("missing.example.org", SystemTime::now()));
        assert_eq!(files(dir.path()).len(), 2);
        assert!(files(dir.path()).iter().all(|f| !f.contains("example")));

        // A half-written file goes; with a fresh key, so do an earlier key's files.
        fs::write(icons.join(format!("{}.tmp", cache.name("example.com").unwrap())), b"..").unwrap();
        let fresh = Cache::with_key(dir.path(), key(2), true);
        assert!(fs::read_dir(&icons).unwrap().next().is_none());
        drop(fresh);

        // Without a key nothing named by host stays, and icons live in memory.
        fs::write(icons.join("example.com"), b"\x89PNG....").unwrap();
        let keyless = Cache::with_key(dir.path(), None, false);
        assert!(!icons.join("example.com").exists());
        keyless.put("example.com", Some(b"\x89PNG....")).unwrap();
        assert_eq!(keyless.get("example.com").unwrap(), b"\x89PNG....");
        assert!(files(dir.path()).iter().all(|f| !f.contains("example")));
    }
}


