//! Exporting the entries in use to a file another password manager imports
//! (#240): CSV as Google Password Manager and Chrome write it, or CXF JSON
//! ([crate::cxf::export]). The reverse of [crate::file_import].
//!
//! Such a file holds every secret in clear: it is built in memory that is
//! wiped when dropped, written only where the user chooses, and never logged.

use crate::edit::{passkey, STANDARD};
use keepass::db::{fields, EntryRef};
use serde::Deserialize;
use zeroize::Zeroizing;

/// The formats a file is written in.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Logins only: `name,url,username,password,note,totp`.
    Csv,
    /// Everything CXF carries, passkeys included.
    Cxf,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Cxf => "json",
        }
    }
}

/// A file's content, and what it could not carry (one line each, for people).
pub struct Export {
    pub content: Zeroizing<String>,
    pub entries: usize,
    pub left_out: Vec<String>,
}

/// The entries as CSV, the columns Google Password Manager and Chrome write
/// (browsers, Bitwarden and 1Password read them) and the TOTP.
pub fn csv<'a>(entries: impl Iterator<Item = EntryRef<'a>>) -> Export {
    let mut content = Zeroizing::new(String::from("name,url,username,password,note,totp\r\n"));
    let (mut count, mut passkeys, mut with_fields, mut with_files) = (0, 0, 0, 0);
    for entry in entries {
        count += 1;
        let get = |name| entry.get(name).unwrap_or("");
        let cells = [get(fields::TITLE), get(fields::URL), get(fields::USERNAME), get(fields::PASSWORD), get(fields::NOTES), get(fields::OTP)];
        for (n, cell) in cells.iter().enumerate() {
            if n > 0 {
                content.push(',');
            }
            push_cell(&mut content, cell);
        }
        content.push_str("\r\n");
        let names: Vec<&str> = entry.fields.keys().map(String::as_str).filter(|n| !STANDARD.contains(n)).collect();
        if names.iter().any(|n| n.starts_with(passkey::PREFIX)) {
            passkeys += 1;
        }
        if names.iter().any(|n| !n.starts_with(passkey::PREFIX)) {
            with_fields += 1;
        }
        if entry.attachments().next().is_some() {
            with_files += 1;
        }
    }
    let mut left_out = Vec::new();
    let note = |n: usize, what: &str| (n > 0).then(|| format!("{what}: {n} {}", if n == 1 { "entry" } else { "entries" }));
    left_out.extend(note(passkeys, "Passkeys (CSV has no place for them; CXF JSON keeps them)"));
    left_out.extend(note(with_fields, "Additional fields"));
    left_out.extend(note(with_files, "Attachments"));
    left_out.push("Groups, tags, icons, expiry and history".into());
    Export { content, entries: count, left_out }
}

/// A CSV cell, quoted when it holds a comma, a quote or a line break.
fn push_cell(out: &mut String, cell: &str) {
    if cell.contains([',', '"', '\n', '\r']) {
        out.push('"');
        for c in cell.chars() {
            if c == '"' {
                out.push('"');
            }
            out.push(c);
        }
        out.push('"');
    } else {
        out.push_str(cell);
    }
}

/// What a CXF export leaves behind (see [crate::cxf::export]).
pub fn cxf_left_out(attachments: usize) -> Vec<String> {
    let mut left_out = Vec::new();
    if attachments > 0 {
        left_out.push(format!("Attachments: {attachments} {}", if attachments == 1 { "entry" } else { "entries" }));
    }
    left_out.push("Icons, expiry and history".into());
    left_out
}

/// A name for the file: `<database> <date>.<extension>`.
pub fn file_name(database: &str, format: Format) -> String {
    let base: String = database.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
    let base = if base.trim().is_empty() { "PswManager".to_string() } else { base.trim().to_string() };
    format!("{base} export {}.{}", chrono::Local::now().format("%Y-%m-%d"), format.extension())
}

#[cfg(test)]
mod tests {
    use super::*;
    use keepass::Database;

    #[test]
    fn csv_reads_back_with_the_importer() {
        let mut db = Database::new();
        let mut root = db.root_mut();
        let mut entry = root.add_entry();
        entry.set_unprotected(fields::TITLE, "Mail, personal");
        entry.set_unprotected(fields::URL, "https://mail.example.com");
        entry.set_unprotected(fields::USERNAME, "alice@example.com");
        entry.set_protected(fields::PASSWORD, "pa\"ss");
        entry.set_unprotected(fields::NOTES, "two\nlines");
        entry.set_protected(passkey::PRIVATE_KEY, "key");
        let export = csv(db.iter_all_entries());
        assert_eq!(export.entries, 1);
        assert!(export.left_out.iter().any(|l| l.starts_with("Passkeys")));
        let import = crate::file_import::parse("export.csv", export.content.as_bytes()).unwrap();
        let data = &import.entries[0].data;
        assert_eq!((data.title.as_str(), data.url.as_str()), ("Mail, personal", "https://mail.example.com"));
        assert_eq!((data.username.as_str(), data.password.as_str()), ("alice@example.com", "pa\"ss"));
        assert_eq!(data.notes, "two\nlines");
    }

    #[test]
    fn cxf_reads_back_with_the_importer() {
        let mut db = Database::new();
        let mut root = db.root_mut();
        let mut entry = root.add_entry();
        entry.set_unprotected(fields::TITLE, "Site");
        entry.set_unprotected(fields::USERNAME, "bob");
        entry.set_protected(fields::PASSWORD, "pw");
        let json = crate::cxf::export(db.iter_all_entries(), "Test", 0);
        let import = crate::file_import::parse("export.json", json.as_bytes()).unwrap();
        let data = &import.entries[0].data;
        assert_eq!((data.title.as_str(), data.username.as_str(), data.password.as_str()), ("Site", "bob", "pw"));
    }

    #[test]
    fn a_file_name_has_no_characters_windows_refuses() {
        let name = file_name("My: db?", Format::Cxf);
        assert!(name.starts_with("My_ db_ export ") && name.ends_with(".json"));
    }
}
