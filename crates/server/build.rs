// Embeds the icon and version details into the .exe when building for Windows.
// On other targets embed-resource does nothing.
fn main() {
    println!("cargo:rerun-if-changed=ingle-server.rc");
    println!("cargo:rerun-if-changed=../app/assets/icon/ingle.ico");
    embed_resource::compile("ingle-server.rc", embed_resource::NONE).manifest_optional().expect("compiling Windows resources");
}
