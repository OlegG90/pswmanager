//! Exporting the entries in use to a file another password manager imports
//! (#240): CSV as Google Password Manager and Chrome write it, or CXF JSON
//! ([crate::cxf::export]). The reverse of [crate::file_import].
//!
//! Such a file holds every secret in clear: it is built in memory that is
//! wiped when dropped, written only where the user chooses, and never logged.

use crate::edit::{passkey, STANDARD};
use crate::otp;
use keepass::db::{fields, EntryRef};
use serde::{Deserialize, Serialize};
use std::path::Path;
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

/// A file's content, the name offered for it, and what it could not carry
/// (one line each, for people).
pub struct Export {
    pub name: String,
    pub content: Zeroizing<String>,
    pub entries: usize,
    pub left_out: Vec<String>,
}

/// What an export wrote, for the window.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    pub file: String,
    pub entries: usize,
    /// What the format could not carry, one line each.
    pub left_out: Vec<String>,
}

impl Export {
    /// Written to `file` (as the user sees it).
    pub fn written(self, file: String) -> Exported {
        Exported { file, entries: self.entries, left_out: self.left_out }
    }
}

/// Writes `content` to a new file at `path` (`shown` in messages), never over
/// a file there nor through a link to one; a file not written whole is removed,
/// as no half a file of secrets is left behind.
pub fn write_new(path: &Path, shown: &str, content: &[u8]) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("{shown} is there already: choose another name"),
        _ => format!("Cannot write {shown}: {e}"),
    })?;
    std::io::Write::write_all(&mut file, content).map_err(|e| {
        let _ = std::fs::remove_file(path);
        format!("Cannot write {shown}: {e}")
    })
}

/// The entries as CSV, the columns Google Password Manager and Chrome write
/// (browsers, Bitwarden and 1Password read them) and the TOTP.
pub fn csv<'a>(entries: impl Iterator<Item = EntryRef<'a>>) -> Export {
    const HEADER: &str = "name,url,username,password,note,totp\r\n";
    const COLUMNS: [&str; 5] = [fields::TITLE, fields::URL, fields::USERNAME, fields::PASSWORD, fields::NOTES];
    let entries: Vec<EntryRef<'a>> = entries.collect();
    // The TOTP in whichever layout the entry keeps it, as more apps read it.
    let totps: Vec<Zeroizing<String>> = entries.iter().map(|e| otp::find(&e.fields).map(|t| otp::portable(&t)).unwrap_or_default()).collect();
    // At its full size at once: a string that grows leaves copies of the
    // secrets it held behind, which nothing wipes.
    let cell_size = |text: &str| 2 * text.len() + 3;
    let most = entries.iter().zip(&totps).map(|(e, totp)| COLUMNS.iter().map(|c| cell_size(e.get(c).unwrap_or(""))).sum::<usize>() + cell_size(totp) + 2).sum::<usize>();
    let mut content = Zeroizing::new(String::with_capacity(HEADER.len() + most));
    content.push_str(HEADER);
    let (mut passkeys, mut with_fields, mut with_files) = (0, 0, 0);
    for (entry, totp) in entries.iter().zip(&totps) {
        let cells = COLUMNS.map(|c| entry.get(c).unwrap_or("")).into_iter().chain([totp.as_str()]);
        for (n, cell) in cells.enumerate() {
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
    Export { name: String::new(), content, entries: entries.len(), left_out }
}

/// A CSV cell, quoted when it holds a comma, a quote or a line break, or
/// starts or ends with a space (which a reader may trim otherwise).
fn push_cell(out: &mut String, cell: &str) {
    if cell.contains([',', '"', '\n', '\r']) || cell.trim() != cell {
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
        entry.set_protected(fields::OTP, "JBSWY3DPEHPK3PXP");
        entry.set_protected(passkey::PRIVATE_KEY, "key");
        let export = csv(db.iter_all_entries());
        assert_eq!(export.entries, 1);
        assert!(export.left_out.iter().any(|l| l.starts_with("Passkeys")));
        let import = crate::file_import::parse("export.csv", export.content.as_bytes()).unwrap();
        let data = &import.entries[0].data;
        assert_eq!((data.title.as_str(), data.url.as_str()), ("Mail, personal", "https://mail.example.com"));
        assert_eq!((data.username.as_str(), data.password.as_str()), ("alice@example.com", "pa\"ss"));
        assert_eq!(data.notes, "two\nlines");
        assert_eq!(data.otp, "JBSWY3DPEHPK3PXP");
        assert!(export.content.len() <= export.content.capacity());
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
