use std::path::Path;

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo::rustc-link-arg=-T{dir}/linker.ld");
    println!("cargo::rerun-if-changed=../../apps/init/src");
    println!("cargo::rerun-if-changed=../../apps/init/Cargo.toml");
    println!("cargo::rerun-if-changed=../../usrlib/linker.ld");
    println!("cargo::rerun-if-changed=../../.cargo/config.toml");
    println!("cargo::rerun-if-changed=../../target/rosx-i486-user/rom.img");

    let rom_path = "../../target/rosx-i486-user/rom.img";
    if !Path::new(rom_path).exists() {
        panic!("missing ROM image {rom_path}; run `cargo xtask build x86_32` first");
    }
    let init_path = "../../target/rosx-i486-user/release/init";
    if !Path::new(init_path).exists() {
        panic!("missing user ELF {init_path}; run `cargo xtask apps x86_32` first");
    }
}
