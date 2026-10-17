// Embeds the icon and version details into the .exe when building for Windows.
// On other targets embed-resource does nothing.
fn main() {
    println!("cargo:rerun-if-changed=pyr.rc");
    println!("cargo:rerun-if-changed=assets/icon/pyr.ico");
    // pyr.rc is a template: the version comes from Cargo.toml, so the file's Properties (and
    // Windows' crash and hang reports) always name the right one.
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let version = std::env::var("CARGO_PKG_VERSION").expect("set by cargo");
    let mut parts: Vec<String> = version.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).take(3).map(String::from).collect();
    parts.resize(4, "0".into());
    let template = std::fs::read_to_string(format!("{dir}/pyr.rc")).expect("reading pyr.rc");
    let rc = template
        .replace("@ICON@", &format!("{dir}/assets/icon/pyr.ico").replace('\\', "/"))
        .replace("@VERSION_COMMAS@", &parts.join(","))
        .replace("@VERSION@", &version);
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("set by cargo")).join("pyr.rc");
    std::fs::write(&out, rc).expect("writing pyr.rc");
    embed_resource::compile(&out, embed_resource::NONE).manifest_optional().expect("compiling Windows resources");
}
