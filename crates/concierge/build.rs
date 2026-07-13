fn main() {
    // rust-embed refuses to compile if the asset folder is missing; make an
    // empty one so `--features webgui` builds even before the first
    // `npm run build`.
    if std::env::var_os("CARGO_FEATURE_WEBGUI").is_some() {
        let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../webgui/dist");
        std::fs::create_dir_all(&dist).expect("cannot create webgui/dist");
        println!("cargo::rerun-if-changed={}", dist.display());
    }
}
