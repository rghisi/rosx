fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let script = std::fs::read(format!("{dir}/linker.ld")).unwrap();
    let hash = script
        .iter()
        .fold(0xcbf29ce484222325u64, |h, byte| (h ^ *byte as u64).wrapping_mul(0x100000001b3));
    println!("cargo::rustc-env=ROSX_LINKER_LD_HASH={hash:016x}");
    println!("cargo::rustc-link-search=native={dir}");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=linker.ld");
}
