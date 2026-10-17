// Embeds the icon and version details into the .exe when building for Windows.
// On other targets embed-resource does nothing.
fn main() {
    println!("cargo:rerun-if-changed=pyr-server.rc");
    println!("cargo:rerun-if-changed=../app/assets/icon/pyr.ico");
    embed_resource::compile("pyr-server.rc", embed_resource::NONE).manifest_optional().expect("compiling Windows resources");
}
