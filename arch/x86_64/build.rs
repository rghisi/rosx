use std::path::Path;

fn main() {
    println!("cargo::rerun-if-changed=../../apps/random_gen_server/src");
    println!("cargo::rerun-if-changed=../../apps/random_gen_server/Cargo.toml");
    println!("cargo::rerun-if-changed=../../apps/random_gen_server/linker.ld");

    let elf_path = "../../target/rosx-user/release/random_gen_server";
    if !Path::new(elf_path).exists() {
        panic!("missing user ELF {elf_path}; run `cargo xtask apps` first");
    }
}
