//! Auto-Type (#272): an entry's user name, password and keys typed into
//! another app's window, as KeePass and KeePassXC do it. Which entries fit a
//! window, which sequence an entry types there, and that sequence read into
//! steps live here; pressing the keys is the Windows app's.
//!
//! The sequences and window associations are KeePass's own (an entry's
//! `AutoType`, a group's `DefaultAutoTypeSequence` and `EnableAutoType`), so
//! what is set in KeePassXC types the same here.

use crate::{edit, icons};
use keepass::db::{fields, EntryRef};
use serde::Deserialize;
use zeroize::Zeroizing;

/// What KeePass and KeePassXC type when nothing else is set.
pub const DEFAULT_SEQUENCE: &str = "{USERNAME}{TAB}{PASSWORD}{ENTER}";

/// The longest pause a sequence may ask for, in milliseconds.
const MAX_DELAY: u32 = 10_000;
/// The most times a key may be repeated (`{TAB 3}`).
const MAX_REPEAT: u32 = 100;

/// The window typed into, as the app read it when the hotkey was pressed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Window {
    pub title: String,
    /// A browser's: it fits entries by its site only, never by its title,
    /// which any page can choose.
    pub browser: bool,
    /// The site a browser shows ([site_of_address]); `None` when its address
    /// could not be read or is not an https page on a named site.
    pub site: Option<String>,
}

/// The site an address read from a browser's address bar is on: an https
/// page on a named host. Browsers leave `https://` out of what they show,
/// so an address without a scheme counts as https; one that says
/// `http://` is no site to type a password into.
pub fn site_of_address(address: &str) -> Option<String> {
    let url = icons::web_url(address)?;
    (url.scheme() == "https").then(|| icons::host_of(url.as_str())).flatten()
}

/// What to type of the entry: its sequence, or one value alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Typed {
    Sequence,
    Username,
    Password,
    Totp,
}

impl Typed {
    /// The sequence that types it into `title`.
    pub fn sequence(self, entry: &EntryRef<'_>, title: &str) -> String {
        match self {
            Typed::Sequence => sequence(entry, title),
            Typed::Username => "{USERNAME}".into(),
            Typed::Password => "{PASSWORD}".into(),
            Typed::Totp => "{TOTP}".into(),
        }
    }
}

/// The keys a sequence names (`{TAB}`); the Windows app knows their codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Tab,
    Enter,
    Space,
    Backspace,
    Delete,
    Escape,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    CapsLock,
    /// F1 to F16.
    F(u8),
}

impl Key {
    fn named(name: &str) -> Option<Key> {
        Some(match name {
            "TAB" => Key::Tab,
            "ENTER" => Key::Enter,
            "SPACE" => Key::Space,
            "BACKSPACE" | "BS" | "BKSP" => Key::Backspace,
            "DELETE" | "DEL" => Key::Delete,
            "ESC" | "ESCAPE" => Key::Escape,
            "INSERT" | "INS" => Key::Insert,
            "HOME" => Key::Home,
            "END" => Key::End,
            "PGUP" => Key::PageUp,
            "PGDN" => Key::PageDown,
            "UP" => Key::Up,
            "DOWN" => Key::Down,
            "LEFT" => Key::Left,
            "RIGHT" => Key::Right,
            "CAPSLOCK" => Key::CapsLock,
            _ => {
                let n: u8 = name.strip_prefix('F')?.parse().ok()?;
                return (1..=16).contains(&n).then_some(Key::F(n));
            }
        })
    }
}

/// The modifiers held for the next key: `+` Shift, `^` Ctrl, `%` Alt, `#` Win.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub win: bool,
}

impl Mods {
    fn any(self) -> bool {
        self.shift || self.ctrl || self.alt || self.win
    }

    /// These and `more` held together.
    fn with(self, more: Mods) -> Mods {
        Mods { shift: self.shift || more.shift, ctrl: self.ctrl || more.ctrl, alt: self.alt || more.alt, win: self.win || more.win }
    }
}

/// One thing to do while typing.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Characters, typed as they are whatever the keyboard layout. A line
    /// break in a value presses Enter, a tab Tab.
    Text(Zeroizing<String>),
    /// A key, with the modifiers held.
    Key(Key, Mods),
    /// The key of a character, with the modifiers held (`^v`).
    Shortcut(char, Mods),
    /// Waits this many milliseconds (`{DELAY 500}`).
    Pause(u32),
    /// Waits this many milliseconds between keys from now on (`{DELAY=50}`).
    KeyDelay(u32),
}

/// A sequence as read, before the entry's values are put in.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Text(String),
    /// A field of the entry, by its name.
    Field(String),
    Totp,
    Key(Key, Mods),
    Shortcut(char, Mods),
    Pause(u32),
    KeyDelay(u32),
}

/// Reads a sequence in KeePass's syntax. A placeholder or command this app
/// does not do is refused as a whole, rather than half of it typed.
fn parse(sequence: &str) -> Result<Vec<Token>, String> {
    /// Adds `token` with the modifiers pressed before it, which then let go.
    fn push(tokens: &mut Vec<Token>, mods: &mut Mods, token: Token) -> Result<(), String> {
        let token = match token {
            // Text comes one character at a time while modifiers are held: one
            // key's, not KeePass's group `^(ab)`.
            Token::Text(text) if mods.any() => match text.chars().next() {
                Some(c) if c != '(' && c != ')' && c.len_utf16() == 1 => Token::Shortcut(c, *mods),
                _ => return Err(format!("A modifier (+ ^ % #) cannot go with {text}")),
            },
            Token::Key(key, own) => Token::Key(key, own.with(*mods)),
            other if mods.any() => return Err(format!("A modifier (+ ^ % #) must come before a key, not before {}", describe(&other))),
            other => other,
        };
        *mods = Mods::default();
        match (tokens.last_mut(), token) {
            (Some(Token::Text(text)), Token::Text(more)) => text.push_str(&more),
            (_, token) => tokens.push(token),
        }
        Ok(())
    }
    let mut tokens = Vec::new();
    let mut mods = Mods::default();
    let mut chars = sequence.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '+' => mods.shift = true,
            '^' => mods.ctrl = true,
            '%' => mods.alt = true,
            '#' => mods.win = true,
            '~' => push(&mut tokens, &mut mods, Token::Key(Key::Enter, Mods::default()))?,
            '{' => {
                // `{}}` is a literal brace: the first `}` is the content.
                let mut inner = String::new();
                if chars.peek() == Some(&'}') {
                    inner.push(chars.next().expect("peeked"));
                }
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => inner.push(c),
                        None => return Err(format!("{{{inner} has no closing }}")),
                    }
                }
                for token in placeholder(&inner)? {
                    push(&mut tokens, &mut mods, token)?;
                }
            }
            '}' => return Err("A } without its {: type it as {}}".into()),
            c => push(&mut tokens, &mut mods, Token::Text(c.to_string()))?,
        }
    }
    if mods.any() {
        return Err("The sequence ends with a modifier (+ ^ % #) and no key".into());
    }
    Ok(tokens)
}

/// How a token is named in a message.
fn describe(token: &Token) -> String {
    match token {
        Token::Field(name) => format!("the field {name}"),
        Token::Totp => "{TOTP}".into(),
        _ => "a delay".into(),
    }
}

/// The tokens one `{…}` stands for.
fn placeholder(inner: &str) -> Result<Vec<Token>, String> {
    let unknown = || format!("Auto-Type cannot do {{{inner}}}");
    // Field names keep their case; the keywords do not.
    if let Some(name) = inner.strip_prefix("S:").or_else(|| inner.strip_prefix("s:")) {
        return Ok(vec![Token::Field(name.to_string())]);
    }
    let upper = inner.trim().to_ascii_uppercase();
    let field = |name: &str| Ok(vec![Token::Field(name.into())]);
    match upper.as_str() {
        "TITLE" => return field(fields::TITLE),
        "USERNAME" => return field(fields::USERNAME),
        "PASSWORD" => return field(fields::PASSWORD),
        "URL" => return field(fields::URL),
        "NOTES" => return field(fields::NOTES),
        "TOTP" | "TIMEOTP" => return Ok(vec![Token::Totp]),
        // Home, then select to the end and delete it, as KeePassXC does.
        "CLEARFIELD" => {
            let ctrl = Mods { ctrl: true, ..Mods::default() };
            return Ok(vec![
                Token::Key(Key::Home, ctrl),
                Token::Key(Key::End, Mods { shift: true, ..ctrl }),
                Token::Key(Key::Backspace, Mods::default()),
            ]);
        }
        "LEFTBRACE" => return Ok(vec![Token::Text("{".into())]),
        "RIGHTBRACE" => return Ok(vec![Token::Text("}".into())]),
        _ => {}
    }
    // A character that means something else in a sequence, typed as itself.
    if let [c @ ('{' | '}' | '+' | '^' | '%' | '~' | '#' | '(' | ')' | '[' | ']')] = inner.chars().collect::<Vec<_>>()[..] {
        return Ok(vec![Token::Text(c.to_string())]);
    }
    let number = |text: &str, max: u32| text.trim().parse::<u32>().ok().filter(|n| *n <= max).ok_or_else(unknown);
    if let Some(ms) = upper.strip_prefix("DELAY=") {
        return Ok(vec![Token::KeyDelay(number(ms, MAX_DELAY)?)]);
    }
    if let Some(ms) = upper.strip_prefix("DELAY ") {
        return Ok(vec![Token::Pause(number(ms, MAX_DELAY)?)]);
    }
    // A key, perhaps repeated: `{TAB 3}`.
    let (name, times) = match upper.split_once(' ') {
        Some((name, times)) => (name, number(times, MAX_REPEAT)?),
        None => (upper.as_str(), 1),
    };
    let key = Key::named(name).ok_or_else(unknown)?;
    Ok(vec![Token::Key(key, Mods::default()); times as usize])
}

/// A sequence as steps, with the entry's values put in: `field` gives a
/// field's value by name (empty when the entry has none), `totp` the current
/// code. Values are typed as they are: placeholders inside them are not read.
pub fn steps(
    sequence: &str,
    field: impl Fn(&str) -> Zeroizing<String>,
    totp: impl Fn() -> Result<Option<String>, String>,
) -> Result<Vec<Step>, String> {
    let mut steps: Vec<Step> = Vec::new();
    let text = |steps: &mut Vec<Step>, more: &str| match steps.last_mut() {
        Some(Step::Text(text)) => text.push_str(more),
        _ => steps.push(Step::Text(Zeroizing::new(more.to_string()))),
    };
    for token in parse(sequence)? {
        match token {
            Token::Text(literal) => text(&mut steps, &literal),
            Token::Field(name) => text(&mut steps, &field(&name)),
            Token::Totp => text(&mut steps, &Zeroizing::new(totp()?.ok_or("The entry has no TOTP secret")?)),
            Token::Key(key, mods) => steps.push(Step::Key(key, mods)),
            Token::Shortcut(c, mods) => steps.push(Step::Shortcut(c, mods)),
            Token::Pause(ms) => steps.push(Step::Pause(ms)),
            Token::KeyDelay(ms) => steps.push(Step::KeyDelay(ms)),
        }
    }
    steps.retain(|s| !matches!(s, Step::Text(t) if t.is_empty()));
    Ok(steps)
}

/// Whether Auto-Type is on for the entry: its own setting, then its groups'
/// (the nearest one that says), on when none does.
pub fn enabled(entry: &EntryRef<'_>) -> bool {
    if entry.autotype.as_ref().is_some_and(|a| !a.enabled) {
        return false;
    }
    let db = entry.database();
    edit::ancestors(db, entry.parent().id()).into_iter().find_map(|g| db.group(g)?.enable_autotype).unwrap_or(true)
}

/// Whether the entry fits a window of another app (not a browser) by its
/// title: the entry's title is part of the window's, or one of its window
/// associations matches it.
pub fn fits_title(entry: &EntryRef<'_>, title: &str) -> bool {
    let own = entry.get(fields::TITLE).map(str::trim).unwrap_or_default();
    (!own.is_empty() && title.to_lowercase().contains(&own.to_lowercase()))
        || entry.autotype.iter().flat_map(|a| &a.associations).any(|a| window_matches(&a.window, title))
}

/// The sequence the entry types into a window titled `title`: that of its
/// first window association that matches (its default sequence when the
/// association has none), else its default sequence, else its groups' (the
/// nearest one that has one), else [DEFAULT_SEQUENCE].
pub fn sequence(entry: &EntryRef<'_>, title: &str) -> String {
    let given = |s: &str| !s.trim().is_empty();
    let own = entry.autotype.as_ref();
    let default = || {
        let db = entry.database();
        own.and_then(|a| a.default_sequence.clone())
            .filter(|s| given(s))
            .or_else(|| edit::ancestors(db, entry.parent().id()).into_iter().find_map(|g| db.group(g)?.default_autotype_sequence.clone().filter(|s| given(s))))
            .unwrap_or_else(|| DEFAULT_SEQUENCE.into())
    };
    match own.and_then(|a| a.associations.iter().find(|x| window_matches(&x.window, title))) {
        Some(association) if given(&association.sequence) => association.sequence.clone(),
        _ => default(),
    }
}

/// Whether a window association's `pattern` matches the window's title,
/// ignoring case: `//…//` is a regular expression; otherwise the whole title,
/// with `*` standing for any text. An empty pattern matches nothing.
pub fn window_matches(pattern: &str, title: &str) -> bool {
    let pattern = pattern.trim();
    if let Some(expression) = pattern.strip_prefix("//").and_then(|p| p.strip_suffix("//")).filter(|e| !e.is_empty()) {
        return regex::RegexBuilder::new(expression).case_insensitive(true).build().is_ok_and(|r| r.is_match(title));
    }
    !pattern.is_empty() && wildcard(&pattern.to_lowercase().chars().collect::<Vec<_>>(), &title.to_lowercase().chars().collect::<Vec<_>>())
}

/// `*` stands for any text; the rest must be as it is.
fn wildcard(pattern: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0, 0);
    // Where the last `*` was, and how much of the text it has taken.
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if p < pattern.len() && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if let Some((at, taken)) = star {
            p = at + 1;
            t = taken + 1;
            star = Some((at, taken + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;
    use keepass::db::{AutoType, AutoTypeAssociation};
    use keepass::Database;

    fn typed(sequence: &str) -> Result<Vec<Step>, String> {
        steps(sequence, |name| Zeroizing::new(format!("<{name}>")), || Ok(Some("123456".into())))
    }

    fn text(t: &str) -> Step {
        Step::Text(Zeroizing::new(t.into()))
    }

    const NONE: Mods = Mods { shift: false, ctrl: false, alt: false, win: false };

    #[test]
    fn the_default_sequence_types_the_user_name_and_password() {
        assert_eq!(
            typed(DEFAULT_SEQUENCE).unwrap(),
            vec![text("<UserName>"), Step::Key(Key::Tab, NONE), text("<Password>"), Step::Key(Key::Enter, NONE)]
        );
    }

    #[test]
    fn keywords_ignore_case_and_text_runs_together() {
        assert_eq!(typed("id: {username}{S:PIN}!~").unwrap(), vec![text("id: <UserName><PIN>!"), Step::Key(Key::Enter, NONE)]);
        assert_eq!(typed("{TOTP}").unwrap(), vec![text("123456")]);
        assert_eq!(typed("{TimeOtp}").unwrap(), vec![text("123456")]);
    }

    #[test]
    fn modifiers_repeats_and_delays() {
        let ctrl = Mods { ctrl: true, ..NONE };
        assert_eq!(typed("^a{DEL}").unwrap(), vec![Step::Shortcut('a', ctrl), Step::Key(Key::Delete, NONE)]);
        assert_eq!(typed("+%{F4}").unwrap(), vec![Step::Key(Key::F(4), Mods { shift: true, alt: true, ..NONE })]);
        assert_eq!(typed("{TAB 3}").unwrap(), (0..3).map(|_| Step::Key(Key::Tab, NONE)).collect::<Vec<_>>());
        assert_eq!(typed("{DELAY 250}{DELAY=40}x").unwrap(), vec![Step::Pause(250), Step::KeyDelay(40), text("x")]);
        assert_eq!(typed("{CLEARFIELD}").unwrap().len(), 3);
    }

    #[test]
    fn special_characters_are_typed_from_braces() {
        assert_eq!(typed("{{}{}}{+}{^}{%}{~}{#}{LEFTBRACE}").unwrap(), vec![text("{}+^%~#{")]);
    }

    #[test]
    fn what_cannot_be_done_is_refused_whole() {
        for bad in ["^(ab)", "^😀", "{CMD:calc}", "{REF:P@I:1}", "{APPACTIVATE x}", "{TAB 1000}", "{DELAY 99999}", "{USERNAME", "}", "^", "^{USERNAME}", "{F17}"] {
            assert!(typed(bad).is_err(), "{bad}");
        }
        let no_code = steps("{TOTP}", |_| Zeroizing::new(String::new()), || Ok(None));
        assert_eq!(no_code.unwrap_err(), "The entry has no TOTP secret");
    }

    #[test]
    fn empty_values_type_nothing() {
        assert_eq!(steps("{USERNAME}{TAB}", |_| Zeroizing::new(String::new()), || Ok(None)).unwrap(), vec![Step::Key(Key::Tab, NONE)]);
    }

    #[test]
    fn window_patterns_take_wildcards_and_regular_expressions() {
        assert!(window_matches("*Notepad", "notes.txt - notepad"));
        assert!(window_matches("Sign in*", "Sign in to Contoso"));
        assert!(window_matches("*bank*", "My Bank — Login"));
        assert!(!window_matches("Notepad", "notes.txt - Notepad"), "without a wildcard the whole title");
        assert!(window_matches("//^Secure Login - .*$//", "secure login - example"));
        assert!(!window_matches("//[//", "anything"), "a broken expression matches nothing");
        assert!(!window_matches("", "anything"));
        assert!(window_matches("*", "anything"));
    }

    #[test]
    fn only_https_sites_from_the_address_bar() {
        assert_eq!(site_of_address("mail.example.com/inbox").as_deref(), Some("mail.example.com"));
        assert_eq!(site_of_address("https://mail.example.com").as_deref(), Some("mail.example.com"));
        assert_eq!(site_of_address("http://mail.example.com"), None);
        assert_eq!(site_of_address("search words"), None);
        assert_eq!(site_of_address("192.168.1.1"), None);
    }

    /// A database with a group `Work` (its own Auto-Type settings) holding one entry.
    fn database(group: impl FnOnce(&mut keepass::db::GroupMut<'_>), entry: impl FnOnce(&mut keepass::db::EntryMut<'_>)) -> Database {
        let mut db = Database::new();
        let mut root = db.root_mut();
        let mut work = root.add_group();
        work.name = "Work".into();
        group(&mut work);
        let mut e = work.add_entry();
        e.set_unprotected(fields::TITLE, "Mail");
        entry(&mut e);
        db
    }

    fn the_entry(db: &Database) -> EntryRef<'_> {
        db.iter_all_entries().next().unwrap()
    }

    fn association(window: &str, sequence: &str) -> AutoTypeAssociation {
        AutoTypeAssociation { window: window.into(), sequence: sequence.into() }
    }

    #[test]
    fn the_sequence_comes_from_the_association_then_the_entry_then_its_groups() {
        let db = database(|g| g.default_autotype_sequence = Some("{PASSWORD}{ENTER}".into()), |_| {});
        assert_eq!(sequence(&the_entry(&db), "x"), "{PASSWORD}{ENTER}");
        let db = database(|_| {}, |_| {});
        assert_eq!(sequence(&the_entry(&db), "x"), DEFAULT_SEQUENCE);
        let db = database(|g| g.default_autotype_sequence = Some("{PASSWORD}".into()), |e| {
            e.autotype = Some(AutoType {
                enabled: true,
                default_sequence: Some("{USERNAME}{ENTER}".into()),
                associations: vec![association("*Login*", "{USERNAME}{TAB}{TAB}{PASSWORD}"), association("*Mail*", "")],
                ..AutoType::default()
            })
        });
        assert_eq!(sequence(&the_entry(&db), "Login — Mail"), "{USERNAME}{TAB}{TAB}{PASSWORD}");
        assert_eq!(sequence(&the_entry(&db), "Mail"), "{USERNAME}{ENTER}", "an association without a sequence");
        assert_eq!(sequence(&the_entry(&db), "Other"), "{USERNAME}{ENTER}");
    }

    #[test]
    fn auto_type_is_off_by_the_entry_or_its_nearest_group() {
        assert!(enabled(&the_entry(&database(|_| {}, |_| {}))));
        assert!(!enabled(&the_entry(&database(|g| g.enable_autotype = Some(false), |_| {}))));
        let off = |e: &mut keepass::db::EntryMut<'_>| e.autotype = Some(AutoType { enabled: false, ..AutoType::default() });
        assert!(!enabled(&the_entry(&database(|_| {}, off))));
        let mut db = database(|g| g.enable_autotype = Some(true), |_| {});
        db.root_mut().enable_autotype = Some(false);
        assert!(enabled(&the_entry(&db)), "the nearest group decides");
    }

    #[test]
    fn an_entry_fits_by_its_title_or_its_associations() {
        let db = database(|_| {}, |e| e.autotype = Some(AutoType { enabled: true, associations: vec![association("Outlook*", "")], ..AutoType::default() }));
        let entry = the_entry(&db);
        assert!(fits_title(&entry, "Inbox - MAIL"));
        assert!(fits_title(&entry, "Outlook — Sign in"));
        assert!(!fits_title(&entry, "Notepad"));
    }
}
