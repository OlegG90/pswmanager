//! PswManager's core, shared by the Windows and Android apps: the KeePass
//! database and its changes, merging, syncing with the stores, TOTP, the
//! generator, password health, similar entries and site icons. Nothing here knows the platform:
//! what does (where secrets are kept, the browser for a sign-in) is handed in.

pub mod backup;
pub mod cxf;
pub mod dbfile;
pub mod documents;
pub mod dropbox;
pub mod edit;
pub mod encryption;
pub mod generator;
pub mod google;
pub mod health;
pub mod icons;
pub mod oauth;
pub mod onedrive;
pub mod opened;
pub mod otp;
pub mod remote;
pub mod secrets;
pub mod session;
pub mod settings;
pub mod similar;
pub mod staged;
pub mod store;
pub mod sync;
pub mod vault;
pub mod webauthn;
