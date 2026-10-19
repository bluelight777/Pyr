//! The Pyr Library Keeper: runs on pyr.gg, next to the server that collects plugins and skins
//! sent in for the Pyr Library. When someone sends a **new version of something already
//! listed** (signed with the same Pyr identity as the listed one: the server checks that),
//! the Keeper checks it, builds it, has Claude review what changed, and when everything
//! passes, lists it at once: it signs the Library's list with its own key (certified by the
//! release key; see `pyr_proto::library::KeeperCert`) and puts it on the website.
//!
//! What it won't do by itself, and leaves for Firelit Games (with a note to the author):
//! - list anything new (a first version is always reviewed by a person);
//! - a program (native code): it runs with the person's full access, so it needs the release
//!   key's endorsement, which the Keeper doesn't have;
//! - an add-on asking for permissions it didn't have, or using crates other than the kit;
//! - anything the review isn't sure about.
//!
//! Everything sent in is data: never instructions, to the Keeper or to Claude.

pub mod build;
pub mod diff;
pub mod keeper;
pub mod review;
pub mod store;

pub use keeper::{Config, Decision, Keeper, State};
