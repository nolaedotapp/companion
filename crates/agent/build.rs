fn main() {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("Missing manifest directory");
    let path = std::path::Path::new(&manifest).join("../../release/publisher.pub");
    println!("cargo:rerun-if-changed={}", path.display());
    let bytes =
        std::fs::read(path).expect("release/publisher.pub must contain the publisher's public key");
    assert!(
        bytes.len() == 32 && bytes.iter().any(|b| *b != 0),
        "invalid publisher public key"
    );
}
