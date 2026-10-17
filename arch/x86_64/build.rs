use std::path::Path;

fn main() {
    println!("cargo::rerun-if-changed=../../apps/random_gen_server/src");
    println!("cargo::rerun-if-changed=../../apps/random_gen_server/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/bulk_echo/src");
    println!("cargo::rerun-if-changed=../../apps/bulk_echo/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/bulk_client/src");
    println!("cargo::rerun-if-changed=../../apps/bulk_client/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/ping_server/src");
    println!("cargo::rerun-if-changed=../../apps/ping_server/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/vfs_server/src");
    println!("cargo::rerun-if-changed=../../apps/vfs_server/Cargo.toml");
    println!("cargo::rerun-if-changed=../../usrlib/linker.ld");
    println!("cargo::rerun-if-changed=../../.cargo/config.toml");

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
    let ping_server_path = "../../target/rosx-user/release/ping_server";
    if !Path::new(&ping_server_path).exists() {
        panic!("missing user ELF {ping_server_path}; run `cargo xtask apps` first");
    }
    let vfs_server_path = "../../target/rosx-user/release/vfs_server";
    if !Path::new(&vfs_server_path).exists() {
        panic!("missing user ELF {vfs_server_path}; run `cargo xtask apps` first");
    }
}
