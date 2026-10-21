fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo::rustc-link-arg=-T{dir}/linker.ld");
    println!("cargo::rerun-if-changed=../../target/rosx-i686-user/release/shell");
    let shell_path = "../../target/rosx-i686-user/release/shell";
    if !std::path::Path::new(shell_path).exists() {
        panic!("missing user ELF {shell_path}; build the x86_32 user ELFs first");
    }
}
