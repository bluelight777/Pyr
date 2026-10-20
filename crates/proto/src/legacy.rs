//! The one place the name Pyr had before 0.8.0 is kept.
//!
//! Pyr had another name until 0.8.0. Nothing people see uses it anymore, but a few hidden
//! identifiers keep it, because changing them would break what's already out there. Code
//! elsewhere refers to these constants instead of spelling the old name out.
//!
//! ## Kept so what's out there keeps working
//! * Invite links made before the rename ([`SCHEME`], [`WEB_INVITE`]).
//! * Settings in the environment ([`ENV_PREFIX`]).
//! * The folders, files and settings copied or moved over on the first start
//!   ([`APP_NAME`] and [`APP_ID`] as directory names, [`LOG_FILES`], [`CONNECTOR`],
//!   [`SSH_KEY`], [`BACKUP_FOLDER`], [`DATA_DIR`], [`DB_FILE`], [`SERVICE`]), and the start
//!   with Windows value ([`APP_NAME`]).
//! * Plugins made with the kit before the rename ([`KIT_CRATE`], and their
//!   `[package.metadata.<APP_ID>]`).
//! * Names nobody but the owner may take on a server ([`RESERVED_NAMES`]).
//!
//! ## Wire labels that never change
//! Every Pyr app, server, plugin and signed release made since the first version uses these
//! byte strings, so they keep the old name for good (changing one would cut new apps off
//! from old servers and the other way round, stop older apps from trusting updates, break
//! every Library plugin, and change everyone's safety codes):
//! * `ALPN` and `CONTROL_PREAMBLE` (lib.rs);
//! * the login, submission, safety, call, session, wrap, media, backup and seal labels
//!   (crypto.rs), the vouch and submission digests (control.rs), the handshake labels
//!   (handshake.rs), the sound digest (sound.rs);
//! * the release manifest and Library list contexts (release.rs, library.rs);
//! * the add-on interface: a plugin exports `<APP_ID>_alloc`, `<APP_ID>_free` and
//!   `<APP_ID>_call` (the plugin kit, and `plugins::addon` in the app).
//!
//! The server's database is `pyr.db` since 0.9.1; a data folder from before has [`DB_FILE`],
//! renamed on the server's first start.

/// The old name, as shown, and as folder names were spelled.
pub const APP_NAME: &str = "Ingle";
/// The old name in lower case: the old URL scheme, folders and identifiers.
pub const APP_ID: &str = "ingle";
/// Invite links from before the rename (`<APP_ID>://…`).
pub const SCHEME: &str = "ingle://";
/// Web invite links from before the rename, and where copies from then look for updates.
pub const WEB_INVITE: &str = "https://firelitgames.com/ingle/";
/// Settings in the environment were `<ENV_PREFIX>NAME`.
pub const ENV_PREFIX: &str = "INGLE_";
/// The app's log files then (left behind when the folder is copied over).
pub const LOG_FILES: &[&str] = &["ingle.log", "ingle.previous.log"];
/// The Claude connector's program then (`<CONNECTOR>` or `<CONNECTOR>.exe`).
pub const CONNECTOR: &str = "ingle-mcp";
/// The SSH key made for rented servers then (the servers it set up still trust it).
pub const SSH_KEY: &str = "ingle_ed25519";
/// Where server backups were kept then (in Documents).
pub const BACKUP_FOLDER: &str = "Ingle Server Backups";
/// The server's data folder when run without `--data`, then.
pub const DATA_DIR: &str = "ingle-data";
/// The server's database file until 0.9.1.
pub const DB_FILE: &str = "ingle.db";
/// The server's service on Linux then (`<SERVICE>.service`, `/var/lib/<SERVICE>`).
pub const SERVICE: &str = "ingle-server";
/// The plugin kit's crate name then.
pub const KIT_CRATE: &str = "ingle-plugin";
/// Names only a server's owner may use, from back then.
pub const RESERVED_NAMES: &[&str] = &["Ingle", "Ingle Library", "Ingle Team"];
