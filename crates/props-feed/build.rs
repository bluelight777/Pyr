//! When this was built (UTC, to the minute). A feed maker put up between two releases of Pyr
//! keeps its version, and this is how its status file tells it from the one before.

fn main() {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    println!("cargo:rustc-env=PYR_PROPS_BUILT={}", secs / 60 * 60);
    // (When the maker itself changes: not every time something else is built.)
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=build.rs");
}
