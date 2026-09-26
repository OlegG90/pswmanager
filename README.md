# PswManager

A minimal tray password manager for Windows on top of a KeePass (KDBX 4) file — in development.
See [docs/spec.md](docs/spec.md) for the MVP specification.

## Development

Requires Node.js, Rust (MSVC toolchain) and Visual Studio Build Tools with the C++ workload.

```
npm install
npx tauri dev            # dev mode
npm test                 # frontend (vitest) and backend unit tests
npm run build            # this machine's architecture: src-tauri/target/release/pswm.exe
```

The app icon is generated from [src-tauri/icons/app-icon.svg](src-tauri/icons/app-icon.svg) with
`npx tauri icon src-tauri/icons/app-icon.svg -o src-tauri/icons` (then delete the non-Windows files).

## sic2kdbx — SafeInCloud XML → KeePass (KDBX 4)

Офлайн-конвертер експорту SafeInCloud у базу KeePass, яку відкривають KeePassXC (ПК) і KeePassDX (Android).

```
python -m venv .venv
.venv\Scripts\python -m pip install -r requirements.txt
.venv\Scripts\python sic2kdbx.py export.xml base.kdbx
```

Скрипт двічі запитає майстер-пароль. Опції: `--keyfile`, `--no-password` (лише файл-ключ), `--skip-deleted` (не переносити видалені картки; за замовчуванням вони йдуть у Кошик), `--drop-empty` (не переносити порожні поля, крім шаблонів), `--force`.

Символи `;` і `,` у назвах міток замінюються пробілом у тегах (KeePass ними розділяє теги); назва групи лишається оригінальною. Пошкоджені вкладення пропускаються з попередженням.

| SafeInCloud | KDBX |
|---|---|
| перше поле login (або e-mail) / password / website / OTP | UserName / Password / URL / otp |
| інші поля | додаткові атрибути; password, pin, secret — захищені |
| notes | Notes |
| мітки | теги; перша мітка — група |
| image, file | вкладення |
| історія полів | історія запису |
| шаблони | група Templates |
| позначка «зірка» | тег Favorite |
| типи полів, порядок, символ, колір | CustomData `SafeInCloud` (JSON) |

Тести: `.venv\Scripts\python -m unittest discover -s tests`

Після конвертації видаліть XML-експорт: у ньому паролі відкритим текстом.
