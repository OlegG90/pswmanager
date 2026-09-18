# PswManager

## sic2kdbx — SafeInCloud XML → KeePass (KDBX 4)

Офлайн-конвертер експорту SafeInCloud у базу KeePass, яку відкривають KeePassXC (ПК) і KeePassDX (Android).

```
python -m venv .venv
.venv\Scripts\python -m pip install -r requirements.txt
.venv\Scripts\python sic2kdbx.py export.xml base.kdbx
```

Скрипт двічі запитає майстер-пароль. Опції: `--keyfile`, `--skip-deleted` (не переносити видалені картки; за замовчуванням вони йдуть у Кошик), `--keep-empty` (зберігати порожні поля), `--force`.

| SafeInCloud | KDBX |
|---|---|
| перше поле login (або e-mail) / password / website / OTP | UserName / Password / URL / otp |
| інші поля | додаткові атрибути; password, pin, secret — захищені |
| notes | Notes |
| мітки | теги; перша мітка — група |
| image, file | вкладення |
| історія полів | історія запису |
| шаблони | група Templates |
| типи полів, порядок, символ, колір | CustomData `SafeInCloud` (JSON) |

Тести: `.venv\Scripts\python -m unittest discover -s tests`

Після конвертації видаліть XML-експорт: у ньому паролі відкритим текстом.
