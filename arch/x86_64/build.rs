use std::path::Path;

fn main() {
    println!("cargo::rerun-if-changed=../../apps/random_gen_server/src");
    println!("cargo::rerun-if-changed=../../apps/random_gen_server/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/random_gen_server/linker.ld");
    println!("cargo::rerun-if-changed=../../apps/bulk_echo/src");
    println!("cargo::rerun-if-changed=../../apps/bulk_echo/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/bulk_echo/linker.ld");
    println!("cargo::rerun-if-changed=../../apps/bulk_client/src");
    println!("cargo::rerun-if-changed=../../apps/bulk_client/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/bulk_client/linker.ld");

    let elf_path = "../../target/rosx-user/release/random_gen_server";
    if !Path::new(elf_path).exists() {
        panic!("missing user ELF {elf_path}; run `cargo xtask apps` first");
    }
    let bulk_echo_path = "../../target/rosx-user/release/bulk_echo";
    if !Path::new(bulk_echo_path).exists() {
        panic!("missing user ELF {bulk_echo_path}; run `cargo xtask apps` first");
    }
    let bulk_client_path = "../../target/rosx-user/release/bulk_client";
    if !Path::new(bulk_client_path).exists() {
        panic!("missing user ELF {bulk_client_path}; run `cargo xtask apps` first");
    }
}
