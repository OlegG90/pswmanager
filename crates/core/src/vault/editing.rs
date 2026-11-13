//! Changes to the entries: saving one from the editor, tags, the recycle
//! bin, duplicating and merging, templates and imports. Each is made and
//! saved through `Vault::change`.

use super::*;

impl Vault {
    /// Makes an older version the entry's current content and saves the file
    /// (see [edit::restore_version]).
    /// `saved` is when the version shown was saved (see [edit::restore_version]).
    pub fn restore_version(&mut self, id: &str, index: usize, saved: Option<&str>) -> Result<(), String> {
        let id = parse_id(id)?;
        self.change(|db, hidden| edit::restore_version(db, id, index, saved, hidden))
    }

    /// The entry with every value, for the editor: not a template or one in
    /// the recycle bin.
    pub fn edit_data(&self, id: &str) -> Option<EntryData> {
        let entry = self.entry(id).filter(|e| kind(e) != Kind::Trash)?;
        Some(edit::read(&entry, group_path(&entry)))
    }

    /// Creates (`id` is `None`) or changes an entry and saves the file.
    /// `base` is the entry as the editor opened it: only what the editor
    /// changed against it is applied, so changes another device made since
    /// stay. Returns the entry's id and the fields both changed (the
    /// editor's version won). Nothing changes when saving fails.
    /// A `template` (a new one, or one being edited) is kept in the templates'
    /// group, made when missing; the editor's group does not move it.
    /// The entry's `files` change with it, as one edit (see [edit::apply_with_files]).
    pub fn save_entry(
        &mut self,
        id: Option<&str>,
        base: Option<&EntryData>,
        data: &EntryData,
        template: bool,
        files: &[edit::FileEdit],
    ) -> Result<(String, Vec<String>), String> {
        let id = id.map(parse_id).transpose()?;
        let (id, conflicts) = self.change(|db, hidden| {
            let current = id.and_then(|id| db.entry(id));
            let template = template || current.as_ref().is_some_and(|e| is_template(e));
            if template && !files.is_empty() {
                return Err("A template's files are not changed".into());
            }
            let mut data = data.clone();
            if template {
                // Where it is now (the top for a new one): nothing moves until
                // it is put among the templates, by the group's id.
                data.group = current.as_ref().map_or_else(Vec::new, group_path);
            }
            let (saved, conflicts) = match (current, base) {
                (Some(entry), Some(base)) => {
                    let (merged, conflicts) = edit::merge3(&edit::read(&entry, group_path(&entry)), base, &data);
                    (edit::apply_with_files(db, id, &merged, files, hidden)?, conflicts)
                }
                _ => (edit::apply_with_files(db, id, &data, files, hidden)?, Vec::new()),
            };
            if template {
                edit::put_among_templates(db, saved)?;
            }
            Ok((saved, conflicts))
        })?;
        Ok((id.uuid().to_string(), conflicts))
    }

    /// Adds the starter templates to a database with no templates (see
    /// [edit::add_starter_templates]) and saves the file.
    pub fn add_starter_templates(&mut self) -> Result<(), String> {
        self.change(|db, _| edit::add_starter_templates(db))
    }

    /// Adds what another password manager exported as new entries with the
    /// tag `tag` (see [crate::cxf]), and saves the file. Returns how many were
    /// added.
    pub fn import(&mut self, import: &crate::cxf::Import, tag: &str) -> Result<usize, String> {
        self.change(|db, hidden| crate::cxf::add(db, import, tag, hidden))
    }

    /// Tidies up the groups older imports made ([crate::cxf::tidy]) and saves
    /// the file. Returns how many entries moved.
    pub fn tidy_imported(&mut self) -> Result<usize, String> {
        self.change(crate::cxf::tidy)
    }

    /// Gives entries a tag or takes it off (the star is the tag Favorite), and
    /// saves the file.
    pub fn set_tag(&mut self, ids: &[String], tag: &str, on: bool) -> Result<(), String> {
        let ids = parse_ids(ids)?;
        self.change(|db, hidden| edit::set_tag(db, &ids, tag, on, hidden))
    }

    /// Renames a tag in every entry and saves the file.
    pub fn rename_tag(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.change(|db, _| edit::rename_tag(db, from, to))
    }

    /// Takes a tag off every entry and saves the file.
    pub fn remove_tag(&mut self, tag: &str) -> Result<(), String> {
        self.change(|db, _| {
            edit::remove_tag(db, tag);
            Ok(())
        })
    }

    /// Puts entries from the recycle bin back where they were and saves the file.
    pub fn restore(&mut self, ids: &[String]) -> Result<(), String> {
        let ids = parse_ids(ids)?;
        self.change(|db, _| edit::restore(db, &ids))
    }

    /// Removes entries in the recycle bin for good and saves the file.
    pub fn delete_for_good(&mut self, ids: &[String]) -> Result<(), String> {
        let ids = parse_ids(ids)?;
        self.change(|db, _| {
            edit::delete_for_good(db, &ids);
            Ok(())
        })
    }

    /// Removes everything in the recycle bin for good and saves the file.
    pub fn empty_bin(&mut self) -> Result<(), String> {
        self.change(|db, _| {
            edit::empty_bin(db);
            Ok(())
        })
    }

    /// Moves entries (templates too) to the recycle bin and saves the file, as
    /// one change. An entry already gone (deleted or binned elsewhere) needs nothing.
    pub fn delete_entries(&mut self, ids: &[String]) -> Result<(), String> {
        let ids = parse_ids(ids)?;
        self.change(|db, _| {
            for &id in &ids {
                if db.entry(id).is_some_and(|entry| kind(&entry) != Kind::Trash) {
                    edit::recycle(db, id)?;
                }
            }
            Ok(())
        })
    }

    /// Copies an entry in use next to it and saves the file (see
    /// [edit::duplicate]); templates and the recycle bin's are not copied.
    /// Returns the copy's id.
    pub fn duplicate(&mut self, id: &str) -> Result<String, String> {
        let id = parse_id(id)?;
        let copy = self.change(|db, _| match db.entry(id) {
            Some(entry) if kind(&entry) == Kind::Entry => edit::duplicate(db, id),
            Some(_) => Err("Only an entry in use is duplicated".into()),
            None => Err(NOT_FOUND.into()),
        })?;
        Ok(copy.uuid().to_string())
    }

    /// Merges entries `others` into `keep` and moves them to the recycle bin
    /// (see [crate::similar::merge]), and saves the file, as one change.
    /// Entries in the recycle bin or templates are not merged.
    pub fn merge_entries(&mut self, keep: &str, others: &[String]) -> Result<(), String> {
        let (keep, others) = (parse_id(keep)?, parse_ids(others)?);
        self.change(|db, hidden| {
            let others = mergeable(db, keep, &others)?;
            crate::similar::merge(db, keep, &others, hidden)
        })
    }

    /// `keep` as [Vault::merge_entries] would leave it, and what it gets:
    /// worked out by merging on a copy, so nothing changes.
    pub fn merge_preview(&self, keep: &str, others: &[String]) -> Result<MergePreview, String> {
        let (keep, others) = (parse_id(keep)?, parse_ids(others)?);
        let others = mergeable(&self.db, keep, &others)?;
        let mut merged = self.db.clone();
        let passkeys_left = crate::similar::merge_noting(&mut merged, keep, &others, &self.hidden_groups())?;
        let (before, after) = (self.db.entry(keep).ok_or(NOT_FOUND)?, merged.entry(keep).ok_or(NOT_FOUND)?);
        Ok(MergePreview { entry: detail_of(&after), added: crate::similar::added(&before, &after), passkeys_left })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::tests::*;
    use crate::edit::{FileChange, FileEdit};

    #[test]
    fn a_merge_preview_shows_the_entry_merged_without_its_secrets() {
        let mut db = Database::new();
        let mut add = |title: &str, user: &str, password: &str| {
            let mut root = db.root_mut();
            let mut e = root.add_entry();
            e.set_unprotected(fields::TITLE, title);
            e.set_unprotected(fields::URL, "https://shop.com");
            e.set_unprotected(fields::USERNAME, user);
            e.set_protected(fields::PASSWORD, password);
            e.id().uuid().to_string()
        };
        let keep = add("Shop", "", "pass-one");
        let other = add("Shop old", "me", "pass-two");
        let vault = Vault::from_database(db);
        let preview = vault.merge_preview(&keep, &[other.clone()]).unwrap();
        assert_eq!(preview.entry.summary.username, "me");
        assert!(preview.added.fields.contains(&fields::USERNAME.to_string()));
        assert_eq!(kind_of(&vault, &other), Some(Kind::Entry), "nothing changes");
        let json = serde_json::to_value(&preview).unwrap();
        let keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["added", "entry", "passkeysLeft"]);
        let text = json.to_string();
        assert!(!text.contains("pass-one") && !text.contains("pass-two"), "{text}");
    }

    #[test]
    fn recycled_entries_show_but_do_not_edit() {
        let db = sample();
        let trash = db.meta.recyclebin_uuid.unwrap();
        let deleted = db.iter_all_entries().find(|e| e.parent().id().uuid() == trash).unwrap().id().uuid().to_string();
        let vault = Vault::from_database(db);
        assert_eq!(vault.detail(&deleted).unwrap().summary.title, "Deleted");
        assert!(vault.edit_data(&deleted).is_none());
        assert!(vault.detail("not-a-uuid").is_none());
    }

    #[test]
    fn a_duplicate_is_a_new_entry_beside_the_original_without_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = sample();
        let mail = db.iter_all_entries().find(|e| e.get(fields::TITLE) == Some("Mail")).unwrap().id();
        db.entry_mut(mail).unwrap().add_attachment("note.txt", Value::unprotected(b"hello".to_vec()));
        db.entry_mut(mail).unwrap().set_icon_custom_new(b"\x89PNG\r\n\x1a\n....".to_vec());
        db.entry_mut(mail).unwrap().edit_tracking(|e| e.set_unprotected(fields::NOTES, "older")); // files a version
        let (db, file) = crate::dbfile::tests::saved(dir.path(), &db);
        let mut vault = Vault { db, file: Some(file), unsaved: false };
        let original = mail.uuid().to_string();
        assert!(vault.detail(&original).unwrap().versions > 0);

        let copy = vault.duplicate(&original).unwrap();
        let (was, now) = (vault.detail(&original).unwrap(), vault.detail(&copy).unwrap());
        assert_ne!(copy, original);
        assert_eq!(now.summary.title, "Mail (Copy)");
        assert_eq!(now.summary.group, was.summary.group);
        assert_eq!(now.summary.tags, was.summary.tags);
        assert_eq!(now.fields, was.fields);
        assert_eq!(now.attachments, was.attachments);
        assert!(now.summary.custom_icon.is_some());
        assert_eq!(now.summary.custom_icon, was.summary.custom_icon, "the image is shared, not stored twice");
        assert_eq!(vault.field(&copy, fields::PASSWORD), vault.field(&original, fields::PASSWORD));
        assert_eq!(now.versions, 0);
        assert!(!vault.has_unsaved());

        let trash = vault.listing().entries.into_iter().find(|e| e.kind == Kind::Trash).unwrap().id;
        assert!(vault.duplicate(&trash).is_err());
        let mut card = EntryData::default();
        card.title = "Card".into();
        let (template, _) = vault.save_entry(None, None, &card, true, &[]).unwrap();
        assert!(vault.duplicate(&template).is_err());
    }

    #[test]
    fn templates_are_made_edited_and_deleted_as_templates() {
        let dir = tempfile::tempdir().unwrap();
        let (db, file) = crate::dbfile::tests::saved(dir.path(), &sample());
        let mut vault = Vault { db, file: Some(file), unsaved: false };
        let mut card = EntryData::default();
        card.title = "Card".into();
        card.group = vec!["Work".into()]; // ignored: a template goes among the templates
        let (id, _) = vault.save_entry(None, None, &card, true, &[]).unwrap();
        // A template's files are not changed.
        let file = FileChange::Add { name: "a.txt".into(), content: Zeroizing::new(b"a".to_vec()) };
        assert!(vault.save_entry(Some(&id), None, &card, false, &[file]).is_err());
        assert_eq!(kind_of(&vault, &id), Some(Kind::Template));
        let templates = vault.db.meta.entry_templates_group.expect("made on demand");
        assert_eq!(vault.db.group(GroupId::from(templates)).unwrap().name, "Templates");

        // Edited like an entry; it stays a template.
        let mut edited = vault.edit_data(&id).unwrap();
        edited.username = "holder".into();
        vault.save_entry(Some(&id), None, &edited, false, &[]).unwrap();
        assert_eq!(kind_of(&vault, &id), Some(Kind::Template));
        assert_eq!(vault.field(&id, fields::USERNAME).unwrap().as_str(), "holder");

        // The editor's group does not move it out.
        edited.group = vec!["Work".into()];
        vault.save_entry(Some(&id), None, &edited, true, &[]).unwrap();
        assert_eq!(kind_of(&vault, &id), Some(Kind::Template));

        // Deleted into the trash, where it is not edited.
        vault.delete_entries(std::slice::from_ref(&id)).unwrap();
        assert_eq!(kind_of(&vault, &id), Some(Kind::Trash));
        assert!(vault.edit_data(&id).is_none());
        // Deleted elsewhere while the editor had it open: saving brings it back.
        vault.save_entry(Some(&id), None, &edited, true, &[]).unwrap();
        assert_eq!(kind_of(&vault, &id), Some(Kind::Template));
    }

    #[test]
    fn history_lists_versions_shows_one_and_restores_it() {
        let dir = tempfile::tempdir().unwrap();
        let (db, file) = crate::dbfile::tests::saved(dir.path(), &sample());
        let mut vault = Vault { db, file: Some(file), unsaved: false };
        let id = in_use(&vault).into_iter().find(|e| e.title == "Mail").unwrap().id;
        let mut data = vault.edit_data(&id).unwrap();
        data.password = "newer".into();
        data.url = "example.org".into();
        vault.save_entry(Some(&id), None, &data, false, &[]).unwrap();
        add_file(&mut vault, &id, "a.txt", b"file");

        assert_eq!(vault.detail(&id).unwrap().versions, 2);
        let history = vault.history(&id).unwrap();
        assert_eq!(history.iter().map(|v| v.changed.clone()).collect::<Vec<_>>(), [vec!["Files"], vec!["Password", "URL"]]);
        assert!(history[1].modified.is_some());
        let json = serde_json::to_string(&history).unwrap();
        assert!(!json.contains("s3cret") && !json.contains("newer"), "{json}");

        // The oldest version: its values on request, how it differs from now.
        let old = vault.version(&id, 1).unwrap();
        assert_eq!(old.detail.summary.url, "example.com/login");
        assert!(old.detail.attachments.is_empty());
        assert_eq!(
            old.differs,
            [
                Difference { name: fields::PASSWORD.into(), current: None, protected: true },
                Difference { name: fields::URL.into(), current: Some("example.org".into()), protected: false },
            ]
        );
        assert_eq!(vault.field_in(&id, Some(1), fields::PASSWORD).unwrap().as_str(), "s3cret");
        assert_eq!(vault.attachment(&id, Some(0), "a.txt"), None);
        assert!(vault.version(&id, 5).is_none());

        // Restored: its content is current again, the replaced one in history.
        // The version shown is checked: the history may have moved on meanwhile.
        assert!(vault.restore_version(&id, 1, Some("2000-01-01T00:00:00Z")).unwrap_err().contains("history changed"));
        vault.restore_version(&id, 1, history[1].modified.as_deref()).unwrap();
        assert_eq!(vault.field(&id, fields::PASSWORD).unwrap().as_str(), "s3cret");
        assert!(vault.detail(&id).unwrap().attachments.is_empty());
        assert_eq!(vault.detail(&id).unwrap().versions, 3);
        assert_eq!(vault.field_in(&id, Some(0), fields::PASSWORD).unwrap().as_str(), "newer");
        assert_eq!(vault.attachment(&id, Some(0), "a.txt").unwrap().as_slice(), b"file");
    }

    #[test]
    fn saving_keeps_everything_the_app_does_not_show() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let before = vault.db.clone();
        vault.file.as_mut().unwrap().save(&mut before.clone()).unwrap();
        vault.db.config.version = keepass::config::DatabaseVersion::KDB4(1);
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(reopened.db, vault.db);
        // Everything but the format version, which saving moves to 4.1.
        let mut expected = before;
        expected.config.version = vault.db.config.version.clone();
        assert_eq!(reopened.db, expected);
    }

    #[test]
    fn an_import_with_times_to_the_millisecond_is_saved() {
        // Bitwarden's times have milliseconds; KDBX keeps whole seconds.
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let json = r#"{"encrypted": false, "items": [{"type": 1, "name": "Site",
            "creationDate": "2026-01-02T03:04:05.123Z", "revisionDate": "2026-01-02T03:04:05.456Z",
            "login": {"username": "u", "password": "p"}}]}"#;
        let import = crate::file_import::parse("bitwarden.json", json.as_bytes()).unwrap();
        assert_eq!(vault.import(&import, "Imported").unwrap(), 1);
    }

    #[test]
    fn an_import_is_saved_and_reads_back_with_its_passkey() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let before = vault.listing().entries.len();
        let import = crate::cxf::parse(
            r#"{"version": {"major": 1, "minor": 0}, "exporterRpId": "example.com", "accounts": [{"id": "YQ",
              "username": "", "email": "", "collections": [], "items": [{"id": "MQ", "title": "Site", "credentials": [
                {"type": "passkey", "credentialId": "Y3JlZA", "rpId": "site.example", "username": "bob",
                 "userDisplayName": "Bob", "userHandle": "dXNlcg", "key": "AAECAwQF"}]}]}]}"#,
        )
        .unwrap();
        assert_eq!(vault.import(&import, "Imported").unwrap(), 1);
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        let listing = reopened.listing();
        assert_eq!(listing.entries.len(), before + 1);
        let site = listing.entries.iter().find(|e| e.title == "Site").unwrap();
        assert!(site.passkey);
        assert!(site.group.is_empty());
        assert_eq!(site.tags, ["Imported"]);
    }

    #[test]
    fn editing_an_entry_keeps_its_attachments_history_and_custom_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let id = vault.listing().entries.iter().find(|e| e.title == "Router").unwrap().id.clone();
        let old = vault.db.entry(EntryId::from(Uuid::parse_str(&id).unwrap())).unwrap().clone();

        let mut data = vault.edit_data(&id).unwrap();
        data.password = "changed".into();
        vault.save_entry(Some(&id), None, &data, false, &[]).unwrap();

        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        let entry = reopened.db.entry(old.id()).unwrap();
        assert_eq!(entry.get_password(), Some("changed"));
        assert_eq!(entry.attachments().count(), 2);
        assert_eq!(entry.custom_data, old.custom_data);
        assert_eq!(entry.history.as_ref().unwrap().get_entries().len(), old.history.as_ref().unwrap().get_entries().len() + 1);
        assert_eq!(entry.get(fields::OTP), old.get(fields::OTP));
        // Templates and the recycle bin stay where they were.
        assert_eq!(in_use(&reopened).len(), 3);
        assert!(std::fs::metadata(dir.path().join("sic2kdbx.kdbx.bak")).is_ok());
    }

    #[test]
    fn attaching_a_file_saves_it_and_keeps_the_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let all_files = |v: &Vault| {
            let mut all: Vec<(String, String, Vec<u8>)> = v
                .db
                .iter_all_entries()
                .flat_map(|e| {
                    let title = e.get_title().unwrap_or_default().to_string();
                    e.attachments_named().map(|(n, a)| (title.clone(), n.to_string(), a.data.get().clone())).collect::<Vec<_>>()
                })
                .collect();
            all.sort();
            all
        };
        let before = all_files(&vault);
        let mail = id_of(&vault, "Mail").uuid().to_string();
        assert!(vault.detail(&mail).unwrap().attachments.is_empty());

        add_file(&mut vault, &mail, "key.txt", b"recovery codes");
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(reopened.detail(&mail).unwrap().attachments, [Attachment { name: "key.txt".into(), size: 14 }]);
        assert_eq!(reopened.attachment(&mail, None, "key.txt").unwrap().as_slice(), b"recovery codes");
        assert!(reopened.attachment(&mail, None, "other.txt").is_none());
        let mut expected = before;
        expected.push(("Mail".into(), "key.txt".into(), b"recovery codes".to_vec()));
        expected.sort();
        assert_eq!(all_files(&reopened), expected);
        // The router's files are listed, by name.
        let router = id_of(&reopened, "Router").uuid().to_string();
        assert_eq!(reopened.detail(&router).unwrap().attachments.len(), 2);
    }

    /// Saves the entry unchanged with these file changes, as the editor does.
    fn change_files(vault: &mut Vault, id: &str, changes: Vec<FileEdit>) {
        let data = vault.edit_data(id).unwrap();
        vault.save_entry(Some(id), None, &data, false, &changes).unwrap();
    }

    fn add_file(vault: &mut Vault, id: &str, name: &str, content: &[u8]) {
        change_files(vault, id, vec![FileChange::Add { name: name.into(), content: Zeroizing::new(content.to_vec()) }]);
    }

    #[test]
    fn removing_a_file_keeps_it_in_history_and_the_other_files_intact() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let router = id_of(&vault, "Router");
        let id = router.uuid().to_string();
        let mail = id_of(&vault, "Mail").uuid().to_string();
        // A file after the router's in the database: it must keep its data
        // when one of the router's is removed.
        add_file(&mut vault, &mail, "later.txt", b"attached later");
        let files = |v: &Vault, id: EntryId| {
            let mut all: Vec<(String, Vec<u8>)> =
                v.db.entry(id).unwrap().attachments_named().map(|(n, a)| (n.to_string(), a.data.get().clone())).collect();
            all.sort();
            all
        };
        let before = files(&vault, router);
        let (gone, kept) = (before[0].clone(), before[1].clone());

        change_files(&mut vault, &id, vec![FileChange::Remove { name: gone.0.clone() }, FileChange::Remove { name: "no such file".into() }]);
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(files(&reopened, router), [kept]);
        assert_eq!(reopened.detail(&id).unwrap().attachments.len(), 1);
        assert_eq!(reopened.attachment(&mail, None, "later.txt").unwrap().as_slice(), b"attached later");
        let previous = reopened.db.entry(router).unwrap().historical(0).unwrap().attachments_named().map(|(n, a)| (n.to_string(), a.data.get().clone())).collect::<Vec<_>>();
        assert!(previous.contains(&gone), "{previous:?}");
        // Attaching and renaming after a removal still number the files right.
        let mut reopened = reopened;
        add_file(&mut reopened, &id, "new.txt", b"new");
        change_files(&mut reopened, &mail, vec![FileChange::Rename { name: "later.txt".into(), to: "renamed.txt".into() }]);
        let again = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(again.attachment(&id, None, "new.txt").unwrap().as_slice(), b"new");
        assert_eq!(again.attachment(&mail, None, "renamed.txt").unwrap().as_slice(), b"attached later");
        assert!(again.attachment(&mail, None, "later.txt").is_none());
        let mut again = again;
        let replaced = Zeroizing::new(b"replaced".to_vec());
        change_files(&mut again, &mail, vec![FileChange::Remove { name: "renamed.txt".into() }, FileChange::Add { name: "renamed.txt".into(), content: replaced }]);
        let last = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(last.attachment(&mail, None, "renamed.txt").unwrap().as_slice(), b"replaced");
        assert_eq!(last.attachment(&id, None, "new.txt").unwrap().as_slice(), b"new");
    }

    #[test]
    fn saving_an_untouched_entry_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let id = in_use(&vault)[0].id.clone();
        let data = vault.edit_data(&id).unwrap();
        vault.save_entry(Some(&id), None, &data, false, &[]).unwrap();
        assert!(!dir.path().join("sic2kdbx.kdbx.bak").exists());
    }

    #[test]
    fn a_chosen_icon_saves_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let mail = id_of(&vault, "Mail").uuid().to_string();
        let router = id_of(&vault, "Router").uuid().to_string();
        let png = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0]);
        for (id, icon) in [(&mail, edit::IconChoice::Custom { data: png }), (&router, edit::IconChoice::Builtin { id: 37 })] {
            let mut data = vault.edit_data(id).unwrap();
            data.icon = icon;
            vault.save_entry(Some(id), None, &data, false, &[]).unwrap();
        }
        let reopened = Vault::open(&path, Some("test"), None).unwrap();
        let listing = reopened.listing();
        let summary = |id: &str| listing.entries.iter().find(|e| e.id == id).unwrap().clone();
        assert!(listing.custom_icons[summary(&mail).custom_icon.as_ref().unwrap()].starts_with("data:image/png"));
        assert_eq!(summary(&router).icon, Some(37));
    }

    #[test]
    fn deleting_saves_and_moves_the_entry_to_the_bin() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let id = in_use(&vault)[0].id.clone();
        vault.delete_entries(std::slice::from_ref(&id)).unwrap();
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(kind_of(&reopened, &id), Some(Kind::Trash));
        assert_eq!(in_use(&reopened).len(), 2);
    }

    #[test]
    fn moving_an_entry_to_another_group_saves() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let router = id_of(&vault, "Router").uuid().to_string();
        let mut data = vault.edit_data(&router).unwrap();
        data.group = vec!["Elsewhere".into()];
        data.password = "and changed".into();
        vault.save_entry(Some(&router), None, &data, false, &[]).unwrap();
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        let detail = reopened.detail(&router).unwrap();
        assert_eq!(detail.summary.group, ["Elsewhere"]);
        assert_eq!(reopened.field(&router, fields::PASSWORD).unwrap().as_str(), "and changed");
    }

    #[test]
    fn a_stale_editor_keeps_the_fields_it_did_not_touch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let router = id_of(&vault, "Router");
        let base = vault.edit_data(&router.uuid().to_string()).unwrap(); // the editor opens
        elsewhere(&path, |db| {
            db.entry_mut(router).unwrap().track_changes().set_unprotected(fields::USERNAME, "changed on the phone");
            let mut root = db.root_mut();
            let mut job = root.add_group();
            job.name = "Job".into();
            let job = job.id();
            db.entry_mut(router).unwrap().move_to(job).unwrap();
        });
        let mut data = base.clone();
        data.password = "changed on the PC".into();
        let (_, conflicts) = vault.save_entry(Some(&router.uuid().to_string()), Some(&base), &data, false, &[]).unwrap();
        assert!(conflicts.is_empty());

        let reopened = Vault::open(&path, Some("test"), None).unwrap();
        let id = router.uuid().to_string();
        assert_eq!(reopened.field(&id, fields::USERNAME).unwrap().as_str(), "changed on the phone");
        assert_eq!(reopened.field(&id, fields::PASSWORD).unwrap().as_str(), "changed on the PC");
        assert_eq!(reopened.detail(&id).unwrap().summary.group, ["Job"]);
    }
}
