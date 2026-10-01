//! Password health: passwords used by more than one entry, weak ones, and
//! ones unchanged for over a year. Checked here only: the window gets titles
//! and reasons, never a password.

use chrono::NaiveDateTime;
use keepass::db::EntryRef;
use serde::Serialize;
use std::collections::HashMap;
use zeroize::Zeroizing;

/// A password this old or older counts as unchanged for too long.
const OLD_AFTER_DAYS: i64 = 365;
/// zxcvbn scores up to this one count as weak ("very weak", "weak").
const WEAK_UP_TO: u8 = 1;
const STRENGTH: [&str; 5] = ["Very weak", "Weak", "Fair", "Strong", "Very strong"];

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Health {
    pub reused: Vec<Finding>,
    /// Not the reused ones: those are listed once, as reused.
    pub weak: Vec<Finding>,
    /// Neither reused nor weak.
    pub old: Vec<Finding>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Finding {
    pub id: String,
    pub title: String,
    /// Why it is listed, e.g. "Same password as Mail, Shop".
    pub detail: String,
}

struct Checked {
    id: String,
    title: String,
    /// A copy, wiped when the check is done.
    password: Zeroizing<String>,
    changed: Option<NaiveDateTime>,
}

/// Checks the entries' passwords; entries without one are left out.
pub fn check<'a>(entries: impl Iterator<Item = EntryRef<'a>>, now: NaiveDateTime) -> Health {
    let mut checked: Vec<Checked> = entries
        .filter_map(|e| {
            let password = Zeroizing::new(e.get_password().filter(|p| !p.is_empty())?.to_string());
            let title = e.get_title().filter(|t| !t.is_empty()).unwrap_or("(no title)").to_string();
            let changed = password_changed(&e, &password);
            Some(Checked { id: e.id().uuid().to_string(), title, password, changed })
        })
        .collect();
    checked.sort_by_cached_key(|c| (c.title.to_lowercase(), c.id.clone()));

    let mut by_password: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, c) in checked.iter().enumerate() {
        by_password.entry(c.password.as_str()).or_default().push(i);
    }

    let mut health = Health::default();
    for (i, c) in checked.iter().enumerate() {
        let finding = |detail: String| Finding { id: c.id.clone(), title: c.title.clone(), detail };
        let same = &by_password[c.password.as_str()];
        if same.len() > 1 {
            let others: Vec<&str> = same.iter().filter(|&&j| j != i).map(|&j| checked[j].title.as_str()).collect();
            health.reused.push(finding(format!("Same password as {}", others.join(", "))));
            continue;
        }
        let score = zxcvbn::zxcvbn(&c.password, &[]).score() as u8;
        if score <= WEAK_UP_TO {
            let length = c.password.chars().count();
            health.weak.push(finding(format!("{} · {length} characters", STRENGTH[score as usize])));
            continue;
        }
        if let Some(changed) = c.changed.filter(|&at| (now - at).num_days() >= OLD_AFTER_DAYS) {
            health.old.push(finding(format!("Last changed {}", changed.format("%-d %b %Y"))));
        }
    }
    health
}

/// When the password was last set: the oldest version in an unbroken run of
/// versions with this password, going back from the current one. A change
/// to anything else (notes, the URL) does not make a password newer.
fn password_changed(entry: &EntryRef, password: &str) -> Option<NaiveDateTime> {
    let mut changed = entry.times.last_modification.or(entry.times.creation);
    // History lists the newest version first.
    let versions = entry.history.as_ref().map(|h| h.get_entries().as_slice()).unwrap_or_default();
    for version in versions {
        if version.get_password() != Some(password) {
            break;
        }
        changed = version.times.last_modification.or(changed);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use keepass::db::{fields, History};
    use keepass::Database;

    fn day(y: i32, m: u32, d: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(12, 0, 0).unwrap()
    }

    const STRONG: &str = "cobalt-Willow-prism-Tundra-88";

    fn add(db: &mut Database, title: &str, password: &str, modified: NaiveDateTime) {
        db.root_mut().add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, title);
            e.set_protected(fields::PASSWORD, password);
            e.times.last_modification = Some(modified);
        });
    }

    fn titles(findings: &[Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.title.as_str()).collect()
    }

    #[test]
    fn finds_reused_weak_and_old_passwords_once_each() {
        let now = day(2026, 9, 27);
        let mut db = Database::new();
        add(&mut db, "Notion", "Summer2023!", day(2025, 1, 3));
        add(&mut db, "Figma", "Summer2023!", day(2025, 1, 3));
        add(&mut db, "Router", "password1", day(2024, 5, 9));
        add(&mut db, "Mail", "vK7#q2Lm!xR9pT4wZe", day(2026, 9, 12));
        add(&mut db, "Bank", STRONG, day(2025, 3, 14));
        add(&mut db, "Empty", "", day(2020, 1, 1));
        let health = check(db.iter_all_entries(), now);

        assert_eq!(titles(&health.reused), ["Figma", "Notion"]);
        assert_eq!(health.reused[0].detail, "Same password as Notion");
        // Weak, and old too: listed once, as weak.
        assert_eq!(titles(&health.weak), ["Router"]);
        assert!(health.weak[0].detail.ends_with("· 9 characters"), "{}", health.weak[0].detail);
        assert_eq!(titles(&health.old), ["Bank"]);
        assert_eq!(health.old[0].detail, "Last changed 14 Mar 2025");
    }

    #[test]
    fn edits_that_keep_the_password_do_not_make_it_newer() {
        let now = day(2026, 9, 27);
        let mut db = Database::new();
        db.root_mut().add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, "Shop");
            e.set_protected(fields::PASSWORD, STRONG);
            e.times.last_modification = Some(day(2026, 9, 1)); // notes changed
            let mut history = History::default();
            let base = (*e).clone();
            let version = |password: &str, at| {
                let mut v = base.clone();
                v.set_protected(fields::PASSWORD, password);
                v.times.last_modification = Some(at);
                v
            };
            // Added oldest first; history lists the newest first.
            history.add_entry(version("old-Password-1", day(2023, 2, 1)));
            history.add_entry(version(STRONG, day(2024, 6, 10)));
            history.add_entry(version(STRONG, day(2025, 8, 1)));
            e.history = Some(history);
        });
        let health = check(db.iter_all_entries(), now);
        assert_eq!(titles(&health.old), ["Shop"]);
        assert_eq!(health.old[0].detail, "Last changed 10 Jun 2024");
    }

    #[test]
    fn a_recent_password_is_healthy() {
        let mut db = Database::new();
        add(&mut db, "Mail", STRONG, day(2026, 9, 1));
        assert_eq!(check(db.iter_all_entries(), day(2026, 9, 27)), Health::default());
    }
}
