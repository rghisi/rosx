fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo::rustc-link-arg=-T{dir}/linker.ld");
    println!("cargo::rustc-link-arg=-z");
    println!("cargo::rustc-link-arg=max-page-size=0x100000");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=linker.ld");
}
