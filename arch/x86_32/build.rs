use std::path::Path;

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo::rustc-link-arg=-T{dir}/linker.ld");
    println!("cargo::rerun-if-changed=../../apps/randomd/src");
    println!("cargo::rerun-if-changed=../../apps/randomd/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/echo/src");
    println!("cargo::rerun-if-changed=../../apps/echo/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/bulk/src");
    println!("cargo::rerun-if-changed=../../apps/bulk/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/pong/src");
    println!("cargo::rerun-if-changed=../../apps/pong/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/vfsd/src");
    println!("cargo::rerun-if-changed=../../apps/vfsd/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/inputd/src");
    println!("cargo::rerun-if-changed=../../apps/inputd/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/ps2d/src");
    println!("cargo::rerun-if-changed=../../apps/ps2d/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/atad/src");
    println!("cargo::rerun-if-changed=../../apps/atad/Cargo.toml");
    println!("cargo::rerun-if-changed=../../usrlib/linker.ld");
    println!("cargo::rerun-if-changed=../../.cargo/config.toml");
    println!("cargo::rerun-if-changed=../../target/rosx-i686-user/rom.img");
    println!("cargo::rerun-if-changed=../../target/rosx-i686-user/release/shell");

    let rom_path = "../../target/rosx-i686-user/rom.img";
    if !Path::new(rom_path).exists() {
        panic!("missing ROM image {rom_path}; run `cargo xtask build x86_32` first");
    }
    for elf in [
        "randomd",
        "echo",
        "bulk",
        "pong",
        "vfsd",
        "inputd",
        "ps2d",
        "atad",
        "shell",
    ] {
        let elf_path = format!("../../target/rosx-i686-user/release/{elf}");
        if !Path::new(&elf_path).exists() {
            panic!("missing user ELF {elf_path}; run `cargo xtask apps x86_32` first");
        }
    }
}
