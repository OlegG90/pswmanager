//! Check for updates: asks GitHub for the repository's latest release, only
//! when the user asks (Settings → About). The request carries nothing but the
//! app's name; nothing is downloaded or installed, the release page is only
//! opened in the browser.

use crate::icons;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// What the check found.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// The latest release's version, without the tag's `v`.
    pub latest: String,
    /// True when it is newer than this copy.
    pub newer: bool,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

/// Asks GitHub's API for the latest release of `repository` (its
/// `https://github.com/owner/name` address) and compares it with `current`.
pub fn check(repository: &str, current: &str) -> Result<UpdateCheck, String> {
    let path = repository.strip_prefix("https://github.com/").ok_or("The repository is not on GitHub")?;
    let url = format!("https://api.github.com/repos/{}/releases/latest", path.trim_end_matches('/'));
    let mut response = icons::http_agent(Duration::from_secs(15), true)
        .get(&url)
        // GitHub refuses requests without a User-Agent.
        .header("User-Agent", "PswManager")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("Cannot check for updates: {e}"))?;
    let body = response.body_mut().read_to_string().map_err(|e| format!("Cannot check for updates: {e}"))?;
    let release: Release = serde_json::from_str(&body).map_err(|_| "GitHub's answer could not be read".to_string())?;
    let latest = release.tag_name.trim_start_matches('v').to_string();
    Ok(UpdateCheck { newer: is_newer(&latest, current), latest })
}

/// `latest` is a later version than `current`, both `X.Y.Z` (a missing part
/// counts as 0; a part that is not a number makes the version unreadable, so
/// never newer).
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parts(latest), parts(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

fn parts(version: &str) -> Option<[u64; 3]> {
    let mut parts = [0; 3];
    for (i, part) in version.trim().trim_start_matches('v').split('.').enumerate() {
        *parts.get_mut(i)? = part.parse().ok()?;
    }
    Some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_version_is_newer() {
        assert!(is_newer("0.16.0", "0.15.0"));
        assert!(is_newer("1.0.0", "0.15.9"));
        assert!(is_newer("0.15.10", "0.15.9"));
        assert!(is_newer("v0.15.1", "0.15.0"));
    }

    #[test]
    fn the_same_or_an_older_version_is_not() {
        assert!(!is_newer("0.15.0", "0.15.0"));
        assert!(!is_newer("0.14.9", "0.15.0"));
        assert!(!is_newer("0.15", "0.15.0"));
    }

    #[test]
    fn an_unreadable_version_is_never_newer() {
        assert!(!is_newer("0.16.0-beta", "0.15.0"));
        assert!(!is_newer("latest", "0.15.0"));
        assert!(!is_newer("1.2.3.4", "0.15.0"));
    }
}
