# sic2kdbx — SafeInCloud XML → KeePass (KDBX 4)

An offline converter from a SafeInCloud export to a KeePass database, which opens in PswManager,
KeePassXC and Keepass2Android. It is a one-off migration tool, not part of the app.

Run it from this folder (`tools/sic2kdbx`); its Python environment lives here too:

```
python -m venv .venv
.venv\Scripts\python -m pip install -r requirements.txt
.venv\Scripts\python sic2kdbx.py export.xml base.kdbx
```

The script asks for the master password twice. Options: `--keyfile`, `--no-password` (key file only),
`--skip-deleted` (leave deleted cards out; by default they go to the recycle bin), `--drop-empty` (leave
empty fields out, except in templates), `--force`.

Elements are written in the order KeePass uses. Databases converted before that change store some
fields out of order; PswManager refuses them rather than risk misreading protected values. Convert again,
or open and save once in KeePassXC.

`;` and `,` in label names become spaces in tags (KeePass separates tags with them); the group name keeps
the original. Damaged attachments are skipped with a warning.

| SafeInCloud | KDBX |
|---|---|
| first login (or e-mail) / password / website / OTP field | UserName / Password / URL / otp |
| other fields | additional attributes; password, pin and secret ones are protected |
| notes | Notes |
| labels | tags; the first label is the group |
| image, file | attachments |
| field history | entry history |
| templates | the Templates group, marked as KeePass's templates group |
| star | the Favorite tag |
| field types, order, symbol, colour | CustomData `SafeInCloud` (JSON) |

Tests: `.venv\Scripts\python -m unittest discover -s tests`

The app's Rust tests use databases this converter makes from the tests' made-up export
(`crates/core/tests/fixtures/`); `.venv\Scripts\python make_fixtures.py` rebuilds them.

After converting, delete the XML export: it holds the passwords in plain text.
