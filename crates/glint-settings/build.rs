fn main() {
    // Refresh the embedded timestamp whenever the settings package is rebuilt.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=build.rs");
    let built_at = chrono::Local::now();
    println!(
        "cargo:rustc-env=GLINT_BUILD_TIME={}",
        built_at.format("%Y%m%d-%H:%M")
    );
    println!("cargo:rerun-if-changed=../../assets/glint.rc");
    println!("cargo:rerun-if-changed=../../assets/glint.ico");
    println!("cargo:rerun-if-changed=../../assets/glint-paused.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("../../assets/glint.rc", embed_resource::NONE)
            .manifest_required()
            .expect("Failed to embed the Glint application icon");
    }
}
