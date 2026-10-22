//! Pyr's poker: the cards, what hands are worth, the rules of a table (Texas Hold'em and
//! Omaha; no limit, pot limit and fixed limit), the pots, a shuffle anyone can check, the bots
//! and the progress a player makes. No networking and no randomness of its own: the Pyr server
//! runs tables with it, and the app uses it to show hands and to check the dealer.

pub mod bacroom;
pub mod bjroom;
pub mod crapsroom;
pub mod bots;
pub mod cards;
pub mod casino;
pub mod eval;
pub mod fair;
pub mod game;
pub mod net;
pub mod progress;
pub mod rating;
pub mod room;
pub mod tourney;

pub use cards::{Card, CardSet};
pub use eval::{Category, Value};
