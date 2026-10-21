# sonora 0.2.0, as Pyr uses it

A copy of [sonora](https://crates.io/crates/sonora) 0.2.0 (BSD-3-Clause, see `LICENSE`), the
Rust port of WebRTC's audio processing, with one addition: `config::EchoCanceller::tune`, a
function that adjusts AEC3's detailed settings (`EchoCanceller3Config`) after the defaults and
before the echo canceller is built. The rest is as published (its examples and benchmarks left
out).

Pyr's echo canceller settings are in `crates/engine/src/audio/capture.rs` (`tune_aec3`), with
why: with the defaults, your voice was turned down 30 to 90 dB for moments at a time while
others talked or music played, headphones or not (bug reports 27 and 32 to 41).
