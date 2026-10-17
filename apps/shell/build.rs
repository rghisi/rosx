use std::path::Path;

fn main() {
    for app in &["hello_elf", "snake", "tetris", "conway"] {
        println!("cargo::rerun-if-changed=../../apps/{app}/src");
        println!("cargo::rerun-if-changed=../../apps/{app}/Cargo.toml");
        println!("cargo::rerun-if-changed=../../apps/{app}/linker.ld");
    }
    println!("cargo::rerun-if-changed=../../apps/ping_client/src");
    println!("cargo::rerun-if-changed=../../apps/ping_client/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/ping_client/linker.ld");
    println!("cargo::rerun-if-changed=../../apps/vfs_client/src");
    println!("cargo::rerun-if-changed=../../apps/vfs_client/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/vfs_client/linker.ld");

    let arch: String = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let base = match arch.as_str() {
        "x86_64" => "../../target/rosx-user/release",
        "x86" => "../../target/rosx-i686-user/release",
        other => panic!("unsupported target arch: {other}"),
    };

    for app in &["hello_elf", "snake", "tetris", "conway"] {
        let elf_path = format!("{base}/{app}");
        if !Path::new(&elf_path).exists() {
            panic!("missing user ELF {elf_path}; run `cargo xtask apps` first");
        }
    }

    if arch == "x86_64" {
        let ping_path = format!("{base}/ping_client");
        if !Path::new(&ping_path).exists() {
            panic!("missing user ELF {ping_path}; run `cargo xtask apps` first");
        }
        let vfs_path = format!("{base}/vfs_client");
        if !Path::new(&vfs_path).exists() {
            panic!("missing user ELF {vfs_path}; run `cargo xtask apps` first");
        }
    }
}
