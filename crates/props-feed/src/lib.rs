//! The props feed's maker (Pyr 0.15): a service on pyr.gg that reads public sources
//! (scoreboards, exchanges' prices, the weather service, earthquakes and the like), makes prop
//! bets from them the way the big prediction sites do, says how each came out, and publishes
//! it all as one signed file that Pyr servers follow (`pyr_proto::props_feed`). Nobody opens,
//! closes or settles any of it by hand: see `docs/PROPS.md`.

pub mod engine;
pub mod http;
pub mod math;
pub mod sources;
pub mod when;
