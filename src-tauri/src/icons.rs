//! Site icons for entries: fetched directly from each site, cached by host in
//! the data folder, never written into the database.

use base64::Engine;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use url::Url;

/// A site that had no icon is asked again after this long.
const RETRY_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const PAGE_LIMIT: u64 = 512 * 1024;
const ICON_LIMIT: u64 = 256 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);

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
    let host = web_url(url)?.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    is_safe_host(&host).then_some(host)
}

/// True for an https address on `host` itself, a subdomain of it, or a parent
/// domain (`www.example.com` -> `example.com`): the site, not someone else.
fn on_site(url: &Url, host: &str) -> bool {
    let Some(other) = url.host_str().map(|h| h.trim_end_matches('.').to_ascii_lowercase()) else { return false };
    let related = other == host
        || other.ends_with(&format!(".{host}"))
        || (other.contains('.') && host.ends_with(&format!(".{other}")));
    url.scheme() == "https" && related
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
    let text_start = || String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).trim_start().to_ascii_lowercase();
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0, 0, 1, 0, ..] => Some("image/x-icon"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ if text_start().starts_with("<svg") || (text_start().starts_with("<?xml") && text_start().contains("<svg")) => {
            Some("image/svg+xml")
        }
        _ => None,
    }
}

/// `bytes` as a `data:` URL for an `<img>`, if they are an image.
pub fn data_url(bytes: &[u8]) -> Option<String> {
    let mime = sniff(bytes)?;
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Candidate icon URLs a page on `host` declares on its own site, best first:
/// icons with a declared size of 32px or more, then Apple touch icons, then
/// the rest.
pub fn icon_links(html: &str, base: &Url, host: &str) -> Vec<Url> {
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
        if !on_site(&url, host) {
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

/// The icon cache: `<dir>/<host>` holds an icon, `<dir>/<host>.miss` marks a
/// site that had none.
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new(dir: PathBuf) -> Self {
        Cache { dir }
    }

    pub fn get(&self, host: &str) -> Option<Vec<u8>> {
        is_safe_host(host).then(|| fs::read(self.dir.join(host)).ok()).flatten()
    }

    /// True when the host has an icon, or was tried recently without one.
    fn is_settled(&self, host: &str, now: SystemTime) -> bool {
        if self.dir.join(host).is_file() {
            return true;
        }
        let missed = fs::metadata(self.dir.join(format!("{host}.miss"))).and_then(|m| m.modified());
        missed.is_ok_and(|t| now.duration_since(t).unwrap_or_default() < RETRY_AFTER)
    }

    fn put(&self, host: &str, icon: Option<&[u8]>) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        match icon {
            Some(bytes) => {
                let tmp = self.dir.join(format!("{host}.tmp"));
                fs::write(&tmp, bytes)?;
                fs::rename(&tmp, self.dir.join(host))?;
                let _ = fs::remove_file(self.dir.join(format!("{host}.miss")));
                Ok(())
            }
            None => fs::write(self.dir.join(format!("{host}.miss")), b""),
        }
    }

    /// Fetches the icons of `hosts` that are not settled yet, calling
    /// `ready(host)` for each one that arrived. Blocking; run it on a thread.
    pub fn fetch_missing(&self, hosts: impl IntoIterator<Item = String>, mut ready: impl FnMut(&str)) {
        let agent = agent();
        for host in hosts {
            if !is_safe_host(&host) || self.is_settled(&host, SystemTime::now()) {
                continue;
            }
            let icon = fetch_icon(&agent, &host);
            if self.put(&host, icon.as_deref()).is_ok() && icon.is_some() {
                ready(&host);
            }
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(true)
        .tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build())
        .build()
        .into()
}

/// Asks the site itself, over https: the icons its start page declares on the
/// same site, then `/favicon.ico`. Only the host is sent — nothing from the
/// entry — and whatever a redirect off the site returns is dropped.
fn fetch_icon(agent: &ureq::Agent, host: &str) -> Option<Vec<u8>> {
    let get = |url: &Url, limit: u64| -> Option<(Url, Vec<u8>)> {
        let mut response = agent.get(url.as_str()).call().ok()?;
        let landed = Url::parse(&ureq::ResponseExt::get_uri(&response).to_string()).ok()?;
        if !on_site(&landed, host) {
            return None;
        }
        Some((landed, response.body_mut().with_config().limit(limit).read_to_vec().ok()?))
    };
    let start = Url::parse(&format!("https://{host}/")).ok()?;
    let mut candidates = match get(&start, PAGE_LIMIT) {
        Some((base, html)) => icon_links(&String::from_utf8_lossy(&html), &base, host),
        None => Vec::new(),
    };
    candidates.push(start.join("/favicon.ico").ok()?);
    candidates.into_iter().find_map(|url| get(&url, ICON_LIMIT).map(|(_, bytes)| bytes).filter(|b| sniff(b).is_some()))
}

/// Where the cache lives: `icons` beside the state file.
pub fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("icons")
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
    fn only_the_site_itself_counts() {
        let on = |url: &str| on_site(&Url::parse(url).unwrap(), "www.example.com");
        assert!(on("https://www.example.com/i.png"));
        assert!(on("https://static.www.example.com/i.png"));
        assert!(on("https://example.com/i.png"));
        assert!(!on("http://www.example.com/i.png"));
        assert!(!on("https://cdn.example.net/i.png"));
        assert!(!on("https://notexample.com/i.png"));
        assert!(!on("https://com/i.png"));
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
            <link rel="icon" href="https://favicons.example.net/example.com">
            <link rel="icon" href="http://example.com/plain.ico">
            <link rel="icon" href="javascript:alert(1)">
        </head></html>"#;
        let urls: Vec<String> = icon_links(html, &base, "example.com").iter().map(Url::to_string).collect();
        assert_eq!(
            urls,
            ["https://cdn.example.com/i/64.png", "https://example.com/app/touch.png", "https://example.com/favicon.ico"]
        );
    }

    #[test]
    fn attributes_are_matched_whole() {
        assert_eq!(attr(r#"<link data-href="x" href="y">"#, "href").as_deref(), Some("y"));
        assert_eq!(attr("<link rel=icon/>", "rel").as_deref(), Some("icon"));
        assert_eq!(attr("<link rel>", "rel"), None);
    }

    #[test]
    fn cache_keeps_icons_and_remembers_misses() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().join("icons"));
        let now = SystemTime::now();
        assert!(!cache.is_settled("example.com", now));
        cache.put("example.com", None).unwrap();
        assert!(cache.is_settled("example.com", now));
        assert!(!cache.is_settled("example.com", now + RETRY_AFTER + Duration::from_secs(1)));
        cache.put("example.com", Some(b"\x89PNG....")).unwrap();
        assert_eq!(cache.get("example.com").unwrap(), b"\x89PNG....");
        assert!(!dir.path().join("icons").join("example.com.miss").exists());
        assert_eq!(cache.get("../pswm.json"), None);
    }
}
