fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "none" {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        println!("cargo::rustc-link-arg=-T{dir}/linker.ld");
        println!("cargo::rustc-link-arg=--pie");
    }
}
