fn main() {
    println!("cargo:rerun-if-changed=../../assets/glint.rc");
    println!("cargo:rerun-if-changed=../../assets/glint.ico");
    println!("cargo:rerun-if-changed=../../assets/glint-paused.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("../../assets/glint.rc", embed_resource::NONE)
            .manifest_required()
            .expect("Failed to embed the Glint application icon");
    }
}
