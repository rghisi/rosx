use std::process::Command;

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out = std::env::var("OUT_DIR").unwrap();
    println!("cargo::rustc-link-arg=-T{dir}/linker.ld");
    println!("cargo::rustc-link-arg=--relax");
    assemble(&dir, &out, "src/boot.S");
    assemble(&dir, &out, "src/swap_context.S");
    println!("cargo::rustc-link-arg={out}/boot.o");
    println!("cargo::rustc-link-arg={out}/swap_context.o");
}

fn assemble(dir: &str, out: &str, src: &str) {
    let name = std::path::Path::new(src).file_stem().unwrap().to_str().unwrap();
    let obj = format!("{out}/{name}.o");
    let status = Command::new("m68k-linux-gnu-as")
        .args(["-m68000", "-o", &obj, &format!("{dir}/{src}")])
        .status()
        .expect("m68k-linux-gnu-as not found; install binutils-m68k-linux-gnu");
    assert!(status.success(), "assembler failed for {src}");
    println!("cargo::rerun-if-changed={dir}/{src}");
}
