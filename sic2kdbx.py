"""Convert a SafeInCloud XML export into a KeePass (KDBX 4) database.

Works fully offline. Mapping:
  card                 -> entry (title, times, password history)
  first login field    -> UserName (falls back to first e-mail field)
  first password field -> Password
  first website field  -> URL
  first OTP field      -> otp (otpauth:// URI, understood by KeePassXC)
  <notes>              -> Notes
  other fields         -> custom string fields (secrets are protected)
  field types, order,
  symbol, color        -> entry CustomData "SafeInCloud" (JSON), for a future app
  labels               -> tags; first label also becomes the group
  <image>, <file>      -> attachments
  template cards       -> "Templates" group
  deleted cards        -> Recycle Bin (or skipped with --skip-deleted)
"""

import argparse
import base64
import getpass
import json
import os
import re
import sys
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from datetime import datetime, timezone
from urllib.parse import quote

from lxml.builder import E
from pykeepass import create_database
from pykeepass.entry import Entry

RESERVED_KEYS = {"Title", "UserName", "Password", "URL", "Notes", "otp"}
STANDARD_KEYS = {"UserName", "Password", "URL"}
PROTECTED_TYPES = {"password", "pin", "secret", "one_time_password", "otp"}
OTP_TYPES = {"one_time_password", "otp", "totp"}
TEMPLATES_GROUP = "Templates"
CUSTOM_DATA_KEY = "SafeInCloud"


@dataclass
class Field:
    name: str
    type: str
    value: str
    history: dict[int, str]  # timestamp (ms) -> value that was set at that moment


@dataclass
class Attachment:
    filename: str
    data: bytes


@dataclass
class Card:
    title: str
    fields: list[Field]
    notes: str
    labels: list[str]
    attachments: list[Attachment]
    template: bool = False
    deleted: bool = False
    star: bool = False
    created: int | None = None
    modified: int | None = None
    extra: dict = field(default_factory=dict)


# ---------------------------------------------------------------- parsing

def _int(value):
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _parse_history(raw):
    if not raw:
        return {}
    try:
        data = json.loads(raw)
    except json.JSONDecodeError:
        return {}
    return {int(ts): val or "" for ts, val in data.items() if str(ts).isdigit()}


def _sniff_extension(data):
    signatures = {
        b"\xff\xd8\xff": ".jpg",
        b"\x89PNG": ".png",
        b"GIF8": ".gif",
        b"RIFF": ".webp",
        b"%PDF": ".pdf",
        b"PK\x03\x04": ".zip",
    }
    for magic, ext in signatures.items():
        if data.startswith(magic):
            return ext
    return ".bin"


def _unique(name, taken):
    if name not in taken:
        return name
    stem, ext = os.path.splitext(name)
    n = 2
    while f"{stem} ({n}){ext}" in taken:
        n += 1
    return f"{stem} ({n}){ext}"


def parse_sic(path):
    root = ET.parse(path).getroot()
    labels = {lbl.get("id"): lbl.get("name") or "" for lbl in root.iter("label")}
    cards = []
    for c in root.iter("card"):
        fields, attachments, names = [], [], set()
        for child in c:
            if child.tag == "field":
                fields.append(Field(
                    name=child.get("name") or child.get("type") or "Field",
                    type=(child.get("type") or "text").lower(),
                    value=child.text or "",
                    history=_parse_history(child.get("history")),
                ))
            elif child.tag in ("image", "file") and (child.text or "").strip():
                try:
                    data = base64.b64decode("".join(child.text.split()), validate=True)
                except ValueError:
                    print(f"Попередження: пошкоджене вкладення <{child.tag}> у картці "
                          f"«{c.get('title')}» пропущено", file=sys.stderr)
                    continue
                name = child.get("name") or f"{child.tag}{_sniff_extension(data)}"
                name = _unique(name, names)
                names.add(name)
                attachments.append(Attachment(name, data))
        notes = c.findtext("notes") or ""
        card_labels = [labels[i.text] for i in c.iter("label_id") if i.text in labels]
        cards.append(Card(
            title=c.get("title") or "",
            fields=fields,
            notes=notes,
            labels=card_labels,
            attachments=attachments,
            template=c.get("template") == "true" or c.get("type") == "template",
            deleted=c.get("deleted") == "true",
            star=c.get("star") == "true",
            created=_int(c.get("first_stamp")),
            modified=_int(c.get("time_stamp")),
            extra={k: c.get(k) for k in ("symbol", "color") if c.get(k)},
        ))
    return cards


# ---------------------------------------------------------------- mapping

def _pick(fields, types, taken):
    """Index of the first non-empty field of the given types, else first of them."""
    candidates = [i for i, f in enumerate(fields) if f.type in types and i not in taken]
    for i in candidates:
        if fields[i].value.strip():
            return i
    return candidates[0] if candidates else None


def assign_keys(card):
    """Map each field index to a KDBX string key."""
    keys, taken = {}, set()
    for types, key in ((("login",), "UserName"), (("password",), "Password"),
                       (("website", "url"), "URL"), (OTP_TYPES, "otp")):
        i = _pick(card.fields, types, taken)
        if key == "UserName" and (i is None or not card.fields[i].value.strip()):
            email = _pick(card.fields, ("email",), taken)
            if email is not None and card.fields[email].value.strip():
                i = email
        if i is not None:
            keys[i] = key
            taken.add(i)
    used = set(RESERVED_KEYS)
    for i, f in enumerate(card.fields):
        if i not in keys:
            keys[i] = _unique(f.name.strip() or f.type, used)
            used.add(keys[i])
    return keys


def _otp_uri(value, title):
    value = value.strip()
    if not value or value.startswith("otpauth://"):
        return value
    secret = value.replace(" ", "").upper()
    return f"otpauth://totp/{quote(title or 'SafeInCloud')}?secret={secret}"


def field_value(f, key, title):
    return _otp_uri(f.value, title) if key == "otp" else f.value


def history_snapshots(card, keys):
    """Reconstruct past states of the card from per-field history.

    SafeInCloud stores {timestamp: value set at that time} per field; the current
    value is the field text. Returns [(timestamp, {key: value})] oldest first,
    skipping repeated states and those where every tracked field is empty
    (SafeInCloud records the blank card at creation). Fields without history
    keep their current value in every snapshot (their past is unknown).
    """
    stamps = sorted({ts for f in card.fields for ts in f.history})
    snapshots, previous = [], None
    for ts in stamps:
        state, tracked = {}, False
        for i, f in enumerate(card.fields):
            if not f.history:
                value = f.value
            else:
                past = [t for t in f.history if t <= ts]
                value = f.history[max(past)] if past else ""
                tracked = tracked or bool(value)
            state[keys[i]] = field_value(Field(f.name, f.type, value, {}), keys[i], card.title)
        if tracked and state != previous:
            snapshots.append((ts, state))
        previous = state
    if snapshots and snapshots[-1][1] == current_values(card, keys):
        snapshots.pop()
    return snapshots


# ---------------------------------------------------------------- writing

def _dt(ms):
    return datetime.fromtimestamp(ms / 1000, tz=timezone.utc)


def _set_custom_data(entry, key, value):
    cd = entry._element.find("CustomData")
    if cd is None:
        cd = E.CustomData()
        entry._element.append(cd)
    cd.append(E.Item(E.Key(key), E.Value(value)))


def _add_string(entry, key, value, protected):
    # Appended directly: pykeepass setters build XPath from the key and break on quotes.
    attrs = {"Protected": "True"} if protected else {}
    entry._element.append(E.String(E.Key(key), E.Value(value, **attrs)))


def _set_fields(entry, card, keys, values, keep_empty):
    """Replace all card-derived string fields of the entry with `values`."""
    for s in entry._element.findall("String"):
        if s.findtext("Key") not in ("Title", "Notes"):
            entry._element.remove(s)
    present = set()
    for i, f in enumerate(card.fields):
        key, value = keys[i], values[keys[i]]
        if not value and key not in STANDARD_KEYS and (
                key == "otp" or not (keep_empty or card.template)):
            continue
        protected = key in ("Password", "otp") or (
            key not in ("UserName", "URL") and f.type in PROTECTED_TYPES)
        _add_string(entry, key, value, protected)
        present.add(key)
    for key in STANDARD_KEYS - present:
        _add_string(entry, key, "", key == "Password")


def current_values(card, keys):
    return {keys[i]: field_value(f, keys[i], card.title) for i, f in enumerate(card.fields)}


def _tag(label):
    # KeePass splits tags on ";" and ",".
    return re.sub(r"\s*[;,]\s*", " ", label).strip()


def add_card(kp, group, card, keep_empty=True):
    keys = assign_keys(card)
    tags = [t for t in dict.fromkeys(
        _tag(label) for label in card.labels + (["Favorite"] if card.star else [])) if t]
    entry = Entry(title=card.title, notes=card.notes or None, tags=tags or None, kp=kp)
    group.append(entry)

    for ts, state in history_snapshots(card, keys):
        _set_fields(entry, card, keys, state, keep_empty)
        entry.mtime = _dt(ts)
        entry.save_history()

    _set_fields(entry, card, keys, current_values(card, keys), keep_empty)

    for att in card.attachments:
        entry.add_attachment(kp.add_binary(att.data), att.filename)

    layout = [{"name": f.name, "type": f.type, "key": keys[i]} for i, f in enumerate(card.fields)]
    _set_custom_data(entry, CUSTOM_DATA_KEY, json.dumps(
        {"fields": layout, **card.extra}, ensure_ascii=False))

    # first_stamp is sometimes the export/sync date, so field history wins if older.
    history_times = [ts for f in card.fields for ts in f.history]
    stamps = [t for t in (card.created, card.modified, *history_times) if t]
    if stamps:
        entry.ctime = _dt(min(stamps))
    if card.modified:
        entry.mtime = _dt(card.modified)
    return entry


def convert(cards, output, password, keyfile=None, skip_deleted=False, keep_empty=True):
    kp = create_database(output, password=password, keyfile=keyfile)
    kp.database_name = "SafeInCloud import"
    kp.root_group.name = "Root"
    groups = {}

    def group_for(name):
        if not name:
            return kp.root_group
        if name not in groups:
            groups[name] = kp.add_group(kp.root_group, name)
        return groups[name]

    stats = {"entries": 0, "deleted": 0, "skipped": 0, "templates": 0, "attachments": 0}
    for card in cards:
        if card.deleted and skip_deleted:
            stats["skipped"] += 1
            continue
        if card.deleted:
            group = kp.root_group  # moved to the Recycle Bin below
        elif card.template:
            group = group_for(TEMPLATES_GROUP)
            stats["templates"] += 1
        else:
            group = group_for(card.labels[0] if card.labels else None)
        entry = add_card(kp, group, card, keep_empty)
        stats["attachments"] += len(card.attachments)
        if card.deleted:
            kp.trash_entry(entry)
            stats["deleted"] += 1
        stats["entries"] += 1


    kp.save()
    return stats


# ---------------------------------------------------------------- CLI

def _read_password(args):
    if args.keyfile and args.no_password:
        return None
    while True:
        first = getpass.getpass("Майстер-пароль нової бази: ")
        if not first:
            print("Пароль не може бути порожнім.", file=sys.stderr)
            continue
        if first == getpass.getpass("Повторіть пароль: "):
            return first
        print("Паролі не збігаються, спробуйте ще раз.", file=sys.stderr)


def main(argv=None):
    for stream in (sys.stdout, sys.stderr):
        stream.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser(description="SafeInCloud XML -> KeePass KDBX 4")
    ap.add_argument("input", help="файл експорту SafeInCloud (.xml)")
    ap.add_argument("output", help="нова база KeePass (.kdbx)")
    ap.add_argument("--keyfile", help="додатковий файл-ключ")
    ap.add_argument("--no-password", action="store_true",
                    help="лише файл-ключ, без майстер-пароля (потрібен --keyfile)")
    ap.add_argument("--skip-deleted", action="store_true",
                    help="не переносити видалені картки (інакше вони йдуть у Кошик)")
    ap.add_argument("--drop-empty", action="store_true",
                    help="не переносити порожні поля карток (крім шаблонів)")
    ap.add_argument("--force", action="store_true", help="перезаписати наявний output")
    args = ap.parse_args(argv)

    if os.path.exists(args.output) and not args.force:
        ap.error(f"{args.output} вже існує (додайте --force для перезапису)")
    if args.no_password and not args.keyfile:
        ap.error("--no-password потребує --keyfile")

    cards = parse_sic(args.input)
    password = _read_password(args)
    stats = convert(cards, args.output, password, args.keyfile,
                    skip_deleted=args.skip_deleted, keep_empty=not args.drop_empty)
    print(f"Готово: {args.output}")
    print(f"  записів: {stats['entries']} (у Кошику: {stats['deleted']}, "
          f"шаблонів: {stats['templates']}), пропущено: {stats['skipped']}, "
          f"вкладень: {stats['attachments']}")


if __name__ == "__main__":
    main()
