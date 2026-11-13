//! The logins an app or a site asks for, and offers to save (the phone's
//! autofill): which entries are for the place asking, and what links an entry
//! to it.

use super::*;

/// Where a login is for: an Android app (its package, and its signing
/// certificate as [crate::webauthn::app_origin] writes it), or a site (the
/// host of the page a browser speaks for).
#[derive(Debug, Clone, PartialEq)]
pub enum LoginPlace {
    App { package: String, certificate: String },
    Site(String),
}

/// The attribute, followed by the app's package, in which an entry keeps the
/// signing certificate of the app it is for. Any app can take any package
/// name; only the app's own signing key gives its certificate.
pub const APP_CERTIFICATE: &str = "PSWM_APP_CERT_";

/// Why an entry for an app's package that keeps no certificate for it (one
/// made in Keepass2Android, say) is not used yet: the user must first confirm
/// the app ([Vault::confirm_app]).
pub const UNCONFIRMED: &str = "PswManager has not given this login to this app before";

/// Why an entry's login was not given to, kept for or linked with the app or
/// site asking.
#[derive(Debug, PartialEq)]
pub enum LoginError {
    /// The entry is for the app's package, and the user has not confirmed the
    /// app yet ([UNCONFIRMED]): the phone asks them.
    Unconfirmed,
    Other(String),
}

impl std::fmt::Display for LoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LoginError::Unconfirmed => UNCONFIRMED,
            LoginError::Other(message) => message,
        })
    }
}

impl From<String> for LoginError {
    fn from(message: String) -> Self {
        LoginError::Other(message)
    }
}

impl From<&str> for LoginError {
    fn from(message: &str) -> Self {
        LoginError::Other(message.into())
    }
}

/// For a caller that only shows why.
impl From<LoginError> for String {
    fn from(error: LoginError) -> Self {
        error.to_string()
    }
}

/// How many entries [Vault::login_candidates] lists at most: enough to pick
/// from, few enough for the phone to show at once.
pub const CANDIDATES: usize = 50;

/// Whether an entry is for a place (see [LoginPlace::fit]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fit {
    /// Another app or site, or the app's package with another certificate.
    No,
    /// For the app's package, but no certificate is kept for it.
    Unconfirmed,
    Yes,
}

impl LoginPlace {
    /// The site a browser speaks for (its page's `origin`), else the app
    /// (`package`, with its signing `certificate`, without which it is unknown).
    pub fn of(origin: Option<&str>, package: &str, certificate: Option<&[u8]>) -> Result<LoginPlace, String> {
        match origin {
            Some(origin) => crate::webauthn::origin_host(origin).map(LoginPlace::Site).ok_or_else(|| "The browser's page is not a site".into()),
            None => {
                let certificate = certificate.ok_or("The app asking is unknown")?;
                Ok(LoginPlace::App { package: package.to_string(), certificate: crate::webauthn::app_origin(certificate) })
            }
        }
    }

    /// Whether the entry is for this place: an app named among its URLs
    /// (`androidapp://<package>`, as Keepass2Android keeps it; packages are
    /// case-sensitive) whose certificate is the one the entry keeps for it
    /// ([Fit::Unconfirmed] while it keeps none), or a site whose host is that
    /// of the entry's URL or of one of its further URLs (`KP2A_URL_n`, #236;
    /// a `www.` aside). Not a site's other hosts: a password goes to its own
    /// hosts only.
    fn fit(&self, entry: &EntryRef<'_>) -> Fit {
        match self {
            LoginPlace::App { package, certificate } => {
                let wanted = format!("androidapp://{package}");
                if !entry.fields.iter().any(|(name, value)| holds_url(name) && value.get().trim() == wanted) {
                    return Fit::No;
                }
                match entry.get(&certificate_field(package)).map(str::trim) {
                    None | Some("") => Fit::Unconfirmed,
                    Some(kept) if kept == certificate => Fit::Yes,
                    Some(_) => Fit::No,
                }
            }
            LoginPlace::Site(host) => {
                let bare = |h: &str| h.trim_start_matches("www.").to_ascii_lowercase();
                let hosts = entry.fields.iter().filter(|(name, _)| holds_url(name)).filter_map(|(_, value)| icons::host_of(value.get()));
                if hosts.map(|site| bare(&site)).any(|site| site == bare(host)) {
                    Fit::Yes
                } else {
                    Fit::No
                }
            }
        }
    }

    /// The URL a new entry for this place gets.
    fn url(&self) -> String {
        match self {
            LoginPlace::App { package, .. } => format!("androidapp://{package}"),
            LoginPlace::Site(host) => format!("https://{host}"),
        }
    }

    /// Keeps an app's certificate in the entry for it, when it keeps none yet
    /// (only for an entry that is for the app, or that the user confirmed it for).
    fn keep_certificate(&self, data: &mut EntryData) {
        let LoginPlace::App { package, certificate } = self else { return };
        let name = certificate_field(package);
        if data.fields.iter().any(|f| f.name == name && !f.value.trim().is_empty()) {
            return;
        }
        data.fields.retain(|f| f.name != name);
        data.fields.push(edit::FieldData { name, value: certificate.clone(), protected: false });
    }
}

/// The attribute an entry keeps the certificate of the app `package` in.
fn certificate_field(package: &str) -> String {
    format!("{APP_CERTIFICATE}{package}")
}

impl Vault {
    /// The entries with a login for `place` (an app or a site), as Credential
    /// Manager offers them to the app asking: an app's own, and those for its
    /// package that keep no certificate yet (used only once the user confirms
    /// the app); not those that keep another certificate.
    pub fn logins_for(&self, place: &LoginPlace) -> Vec<EntryChoice> {
        self.places_entries(place, true)
    }

    /// Entry `id`'s user name and password, to hand to the app or site
    /// asking: only when the entry is for it; [LoginError::Unconfirmed] while it
    /// is for the app's package and the user has not confirmed the app.
    pub fn login(&self, id: &str, place: &LoginPlace) -> Result<(Zeroizing<String>, Zeroizing<String>), LoginError> {
        let entry = self.entry_for(id, place, false)?;
        let text = |name: &str| Zeroizing::new(entry.get(name).unwrap_or_default().to_string());
        Ok((text(fields::USERNAME), text(fields::PASSWORD)))
    }

    /// The user confirmed that the app asking is the one entry `id` (for its
    /// package) is for: the entry keeps the app's certificate, so only this
    /// app gets it from now on; the file is saved (the old version goes to
    /// history). Nothing changes for an entry that keeps it already, or a site.
    pub fn confirm_app(&mut self, id: &str, place: &LoginPlace) -> Result<(), String> {
        self.entry_for(id, place, true)?;
        let base = self.edit_data(id).ok_or(NOT_FOUND)?;
        let mut data = base.clone();
        place.keep_certificate(&mut data);
        if data == base {
            return Ok(());
        }
        self.save_entry(Some(id), Some(&base), &data, false, &[]).map(|_| ())
    }

    /// Entry `id`, an entry in use for `place`; [LoginError::Unconfirmed] for
    /// one for the app's package that keeps no certificate, unless the user
    /// `confirmed` the app.
    fn entry_for(&self, id: &str, place: &LoginPlace, confirmed: bool) -> Result<EntryRef<'_>, LoginError> {
        let entry = self.entry(id).filter(|e| kind(e) == Kind::Entry).ok_or(NOT_FOUND)?;
        match place.fit(&entry) {
            Fit::Yes => Ok(entry),
            Fit::Unconfirmed if confirmed => Ok(entry),
            Fit::Unconfirmed => Err(LoginError::Unconfirmed),
            Fit::No => Err("This login is not for the app asking".into()),
        }
    }

    /// The entries a login an app offers to save can update: those for
    /// `place`, as [Vault::logins_for] has them.
    pub fn entries_for_login(&self, place: &LoginPlace) -> Vec<EntryChoice> {
        self.places_entries(place, false)
    }

    /// The user's entries for `place`; only those with a password when `with_password`.
    fn places_entries(&self, place: &LoginPlace, with_password: bool) -> Vec<EntryChoice> {
        self.visible_entries()
            .filter(|e| place.fit(e) != Fit::No && (!with_password || e.get(fields::PASSWORD).is_some_and(|p| !p.is_empty())))
            .map(|e| EntryChoice::of(&e, e.get(fields::USERNAME)))
            .collect()
    }

    /// Keeps a login an app offers to save: the password (and the user name
    /// when the entry has none) of entry `id`, which must be for `place` (as
    /// [Vault::login] checks, the user having `confirmed` the app for an entry
    /// that keeps no certificate), or a new entry titled `title` for `place`.
    /// An app's entry keeps its certificate. The file is saved (the old
    /// version goes to history). Returns the entry's id.
    pub fn save_login(&mut self, id: Option<&str>, place: &LoginPlace, confirmed: bool, title: &str, username: &str, password: &str) -> Result<String, LoginError> {
        if password.is_empty() {
            return Err("There is no password to save".into());
        }
        let (base, mut data) = match id {
            Some(id) => {
                self.entry_for(id, place, confirmed)?;
                let data = self.edit_data(id).ok_or(NOT_FOUND)?;
                (Some(data.clone()), data)
            }
            None => {
                let mut data = EntryData::default();
                data.title = title.to_string();
                data.url = place.url();
                (None, data)
            }
        };
        if data.username.is_empty() {
            data.username = username.to_string();
        }
        data.password = password.to_string();
        place.keep_certificate(&mut data);
        Ok(self.save_entry(id, base.as_ref(), &data, false, &[])?.0)
    }

    /// The entries in use whose title, user name or one of whose URLs holds
    /// `query` (any case; all of them for an empty one), for the user to pick
    /// one for a login an app or site asks for or offers when none is for it
    /// (#270); those with a password first, then by title. At most
    /// [CANDIDATES] of them.
    pub fn login_candidates(&self, query: &str) -> Vec<EntryChoice> {
        let query = query.trim().to_lowercase();
        let holds = |text: &str| text.to_lowercase().contains(&query);
        let mut found: Vec<(bool, EntryChoice)> = self
            .visible_entries()
            .filter(|e| kind(e) == Kind::Entry)
            .filter(|e| {
                query.is_empty()
                    || [fields::TITLE, fields::USERNAME].iter().any(|name| e.get(name).is_some_and(holds))
                    || e.fields.iter().any(|(name, value)| holds_url(name) && holds(value.get()))
            })
            .map(|e| (e.get(fields::PASSWORD).is_some_and(|p| !p.is_empty()), EntryChoice::of(&e, e.get(fields::USERNAME))))
            .collect();
        found.sort_by(|(a_password, a), (b_password, b)| b_password.cmp(a_password).then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase())));
        found.into_iter().take(CANDIDATES).map(|(_, choice)| choice).collect()
    }

    /// Links entry `id`, which the user picked, to `place` (#270), as
    /// Keepass2Android does: the place's URL becomes the entry's URL when it
    /// has none, else its next further URL (`KP2A_URL_n`), and an app's entry
    /// keeps the app's certificate. For an app that must be `confirmed` first
    /// ([UNCONFIRMED] till then): linking hands the entry to it. Nothing
    /// changes for an entry that is for the place already; an entry that keeps
    /// another certificate for the app's package is not linked. The file is
    /// saved (the old version goes to history).
    pub fn link_login(&mut self, id: &str, place: &LoginPlace, confirmed: bool) -> Result<(), LoginError> {
        let entry = self.entry(id).filter(|e| kind(e) == Kind::Entry).ok_or(NOT_FOUND)?;
        let fit = place.fit(&entry);
        if fit == Fit::Yes {
            return Ok(());
        }
        if let LoginPlace::App { package, .. } = place {
            if entry.get(&certificate_field(package)).is_some_and(|kept| !kept.trim().is_empty()) {
                return Err("This entry is for another app of that name".into());
            }
            if !confirmed {
                return Err(LoginError::Unconfirmed);
            }
        }
        let base = self.edit_data(id).ok_or(NOT_FOUND)?;
        let mut data = base.clone();
        if fit == Fit::No {
            let url = place.url();
            if data.url.trim().is_empty() {
                data.url = url;
            } else {
                let taken = |n: &usize| data.fields.iter().any(|f| f.name == format!("{}_{n}", crate::cxf::MORE_URLS));
                let n = (1..).find(|n| !taken(n)).unwrap_or(1);
                data.fields.push(edit::FieldData { name: format!("{}_{n}", crate::cxf::MORE_URLS), value: url, protected: false });
            }
        }
        place.keep_certificate(&mut data);
        self.save_entry(Some(id), Some(&base), &data, false, &[])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::tests::*;

    #[test]
    fn logins_are_offered_to_their_app_or_site_and_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let app = LoginPlace::of(None, "com.example.mail", Some(b"mail's certificate")).unwrap();
        let site = LoginPlace::Site("www.mail.example".into());
        assert!(vault.logins_for(&app).is_empty());

        // A new login for the app, then offered to it (and not to others).
        let id = vault.save_login(None, &app, false, "Mail", "alice", "pw-1").unwrap();
        let offered = vault.logins_for(&app);
        assert_eq!((offered[0].id.as_str(), offered[0].username.as_str()), (id.as_str(), "alice"));
        let other_app = |package: &str| LoginPlace::of(None, package, Some(b"mail's certificate")).unwrap();
        assert!(vault.logins_for(&other_app("com.example.other")).is_empty());
        assert!(vault.logins_for(&other_app("com.example.MAIL")).is_empty());
        let (user, password) = vault.login(&id, &app).unwrap();
        assert_eq!((user.as_str(), password.as_str()), ("alice", "pw-1"));

        // An app's changed password updates the entry; the user name stays.
        assert_eq!(vault.entries_for_login(&app).len(), 1);
        vault.save_login(Some(&id), &app, false, "", "someone", "pw-2").unwrap();
        let (user, password) = vault.login(&id, &app).unwrap();
        assert_eq!((user.as_str(), password.as_str()), ("alice", "pw-2"));

        // A site's login is offered to its own host (a www. aside), not to the site's other hosts.
        let web = vault.save_login(None, &LoginPlace::Site("mail.example".into()), false, "Mail web", "bob", "pw-3").unwrap();
        assert_eq!(vault.logins_for(&site).iter().map(|c| c.id.clone()).collect::<Vec<_>>(), std::slice::from_ref(&web));
        assert!(vault.logins_for(&LoginPlace::Site("evil.mail.example".into())).is_empty());
        assert!(vault.login(&web, &LoginPlace::Site("evil.mail.example".into())).is_err());
        assert!(vault.login(&web, &app).is_err());
        assert!(vault.login(&id, &site).is_err());
        assert!(vault.save_login(Some(&web), &app, true, "", "", "pw-4").is_err());
        assert!(vault.save_login(None, &app, false, "Mail", "alice", "").is_err());
        assert_eq!(LoginPlace::of(Some("https://www.mail.example"), "com.android.chrome", None).unwrap(), LoginPlace::Site("www.mail.example".into()));
        let mail = LoginPlace::App { package: "com.example.mail".into(), certificate: crate::webauthn::app_origin(b"mail's certificate") };
        assert_eq!(app, mail);
        assert!(LoginPlace::of(None, "com.example.mail", None).is_err());
    }

    #[test]
    fn a_site_finds_its_logins_by_further_urls_too() {
        let mut db = Database::new();
        db.root_mut().add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, "Mail");
            e.set_unprotected(fields::URL, "https://mail.example");
            e.set_unprotected("KP2A_URL_1", "https://login.mail.example/signin");
            e.set_unprotected(fields::USERNAME, "alice");
            e.set_unprotected(fields::PASSWORD, "pw");
        });
        let vault = Vault::from_database(db);
        for host in ["mail.example", "login.mail.example", "www.login.mail.example"] {
            assert_eq!(vault.logins_for(&LoginPlace::Site(host.into())).len(), 1, "{host}");
        }
        assert!(vault.logins_for(&LoginPlace::Site("other.mail.example".into())).is_empty());
    }

    #[test]
    fn any_entry_can_be_picked_and_linked_to_an_app_or_site() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let photos = LoginPlace::of(None, "com.example.photos", Some(b"photos' certificate")).unwrap();
        let site = LoginPlace::Site("photos.example".into());
        let mut data = EntryData::default();
        data.title = "Photos".into();
        data.username = "alice".into();
        data.password = "pw-1".into();
        data.url = "https://photos.home.example".into();
        let (id, _) = vault.save_entry(None, None, &data, false, &[]).unwrap();
        let mut bare = EntryData::default();
        bare.title = "Photos note".into();
        let (note, _) = vault.save_entry(None, None, &bare, false, &[]).unwrap();
        assert!(vault.logins_for(&photos).is_empty());

        // Found by title, user name or URL, any case; one with a password first.
        let ids = |query: &str| vault.login_candidates(query).into_iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids("PHOTOS"), [id.clone(), note.clone()]);
        assert_eq!(ids("alice"), std::slice::from_ref(&id));
        assert_eq!(ids("home.example"), std::slice::from_ref(&id));
        assert!(ids("nothing like it").is_empty());
        assert!(vault.login_candidates("").len() > 2);

        // An app is linked only once the user confirmed it: the entry gets its
        // URL as a further one, and its certificate.
        assert_eq!(vault.link_login(&id, &photos, false).unwrap_err(), LoginError::Unconfirmed);
        vault.link_login(&id, &photos, true).unwrap();
        let linked = vault.edit_data(&id).unwrap();
        assert_eq!(linked.url, "https://photos.home.example");
        assert!(linked.fields.iter().any(|f| f.name == "KP2A_URL_1" && f.value == "androidapp://com.example.photos"));
        assert_eq!(vault.logins_for(&photos).len(), 1);
        assert_eq!(vault.login(&id, &photos).unwrap().1.as_str(), "pw-1");
        assert_eq!(vault.history(&id).unwrap().len(), 1);
        // Linked already: nothing changes.
        vault.link_login(&id, &photos, false).unwrap();
        assert_eq!(vault.history(&id).unwrap().len(), 1);
        // Another app of the same name is not linked to it.
        let impostor = LoginPlace::of(None, "com.example.photos", Some(b"another certificate")).unwrap();
        assert!(vault.link_login(&id, &impostor, true).is_err());

        // A site goes into the next further URL; an entry with no URL gets it as its URL.
        vault.link_login(&id, &site, false).unwrap();
        assert!(vault.edit_data(&id).unwrap().fields.iter().any(|f| f.name == "KP2A_URL_2" && f.value == "https://photos.example"));
        vault.link_login(&note, &site, false).unwrap();
        assert_eq!(vault.edit_data(&note).unwrap().url, "https://photos.example");
        assert_eq!(vault.logins_for(&site).len(), 1);
        assert_eq!(vault.entries_for_login(&site).len(), 2);
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(reopened.logins_for(&photos).len(), 1);
    }

    #[test]
    fn an_app_gets_a_login_only_with_the_certificate_the_entry_keeps() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let bank = LoginPlace::of(None, "com.example.bank", Some(b"the bank's certificate")).unwrap();
        let impostor = LoginPlace::of(None, "com.example.bank", Some(b"another certificate")).unwrap();
        // An entry for the app made elsewhere, as Keepass2Android keeps it: no certificate.
        let mut data = EntryData::default();
        data.title = "Bank".into();
        data.username = "alice".into();
        data.password = "pw-1".into();
        data.fields = vec![edit::FieldData { name: "KP2A_URL_1".into(), value: "androidapp://com.example.bank".into(), protected: false }];
        let (id, _) = vault.save_entry(None, None, &data, false, &[]).unwrap();

        // Offered to any app by that name, but nothing is handed over or
        // replaced until the user confirms the app.
        assert_eq!(vault.logins_for(&bank).len(), 1);
        assert_eq!(vault.logins_for(&impostor).len(), 1);
        assert_eq!(vault.login(&id, &impostor).unwrap_err(), LoginError::Unconfirmed);
        assert_eq!(vault.save_login(Some(&id), &impostor, false, "", "", "pw-x").unwrap_err(), LoginError::Unconfirmed);
        assert_eq!(vault.field(&id, fields::PASSWORD).unwrap().as_str(), "pw-1");

        // Confirmed, the entry keeps the bank app's certificate: from then on
        // only that app gets the login or can change it.
        vault.confirm_app(&id, &bank).unwrap();
        let (user, password) = vault.login(&id, &bank).unwrap();
        assert_eq!((user.as_str(), password.as_str()), ("alice", "pw-1"));
        assert!(vault.logins_for(&impostor).is_empty());
        assert!(vault.entries_for_login(&impostor).is_empty());
        assert!(vault.login(&id, &impostor).is_err());
        assert!(vault.confirm_app(&id, &impostor).is_err());
        assert!(vault.save_login(Some(&id), &impostor, true, "", "", "pw-x").is_err());
        vault.save_login(Some(&id), &bank, false, "", "", "pw-2").unwrap();
        assert_eq!(vault.login(&id, &bank).unwrap().1.as_str(), "pw-2");

        // The certificate is an ordinary attribute in the file; confirming again changes nothing.
        let name = format!("{APP_CERTIFICATE}com.example.bank");
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(reopened.field(&id, &name).unwrap().as_str(), crate::webauthn::app_origin(b"the bank's certificate"));
        let versions = vault.history(&id).unwrap().len();
        vault.confirm_app(&id, &bank).unwrap();
        assert_eq!(vault.history(&id).unwrap().len(), versions);

        // Saving into such an entry once the user confirmed the app keeps its certificate too.
        let mut data = EntryData::default();
        data.title = "Shop".into();
        data.password = "pw-3".into();
        data.url = "androidapp://com.example.shop".into();
        let (shop, _) = vault.save_entry(None, None, &data, false, &[]).unwrap();
        let shop_app = LoginPlace::of(None, "com.example.shop", Some(b"the shop's certificate")).unwrap();
        vault.save_login(Some(&shop), &shop_app, true, "", "bob", "pw-4").unwrap();
        assert_eq!(vault.login(&shop, &shop_app).unwrap().1.as_str(), "pw-4");
        assert!(vault.logins_for(&LoginPlace::of(None, "com.example.shop", Some(b"x")).unwrap()).is_empty());
    }
}
