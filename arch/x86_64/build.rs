use std::path::Path;

fn main() {
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
    println!("cargo::rerun-if-changed=../../usrlib/linker.ld");
    println!("cargo::rerun-if-changed=../../.cargo/config.toml");
    println!("cargo::rerun-if-changed=../../target/rosx-user/rom.img");
    println!("cargo::rerun-if-changed=../../target/rosx-user/release/shell");

    let elf_path = "../../target/rosx-user/release/randomd";
    if !Path::new(elf_path).exists() {
        panic!("missing user ELF {elf_path}; run `cargo xtask apps` first");
    }
    let echo_path = "../../target/rosx-user/release/echo";
    if !Path::new(echo_path).exists() {
        panic!("missing user ELF {echo_path}; run `cargo xtask apps` first");
    }
    let bulk_path = "../../target/rosx-user/release/bulk";
    if !Path::new(bulk_path).exists() {
        panic!("missing user ELF {bulk_path}; run `cargo xtask apps` first");
    }
    let pong_path = "../../target/rosx-user/release/pong";
    if !Path::new(pong_path).exists() {
        panic!("missing user ELF {pong_path}; run `cargo xtask apps` first");
    }
    let vfsd_path = "../../target/rosx-user/release/vfsd";
    if !Path::new(&vfsd_path).exists() {
        panic!("missing user ELF {vfsd_path}; run `cargo xtask apps` first");
    }
    let rom_path = "../../target/rosx-user/rom.img";
    if !Path::new(rom_path).exists() {
        panic!("missing ROM image {rom_path}; run `cargo xtask build` first");
    }
    let shell_path = "../../target/rosx-user/release/shell";
    if !Path::new(shell_path).exists() {
        panic!("missing user ELF {shell_path}; run `cargo xtask apps` first");
    }
}
