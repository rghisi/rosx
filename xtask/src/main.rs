use std::env;
use std::path::PathBuf;
use std::process::{self, Command};

mod arch;

use arch::{ArchSpec, ImageKind, X86_64};

const USER_APPS: [&str; 20] = ["pi", "randomd", "echo", "bulk", "pong", "ping", "mem", "vfsd", "vfs", "snake", "tetris", "conway", "threads", "waitset", "inputd", "ps2d", "stress", "bench", "atad", "ata"];

const RUN_DISK_BYTES: usize = 1024 * 1024;
const MBR_SIGNATURE_OFFSET: usize = 510;

const UNSTABLE: [&str; 3] = [
    "-Zbuild-std=core,alloc,compiler_builtins",
    "-Zbuild-std-features=compiler-builtins-mem",
    "-Zjson-target-spec",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn cargo() -> Command {
    let mut cmd = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(workspace_root());
    cmd
}

fn run(cmd: &mut Command, description: &str) {
    println!("==> {description}");
    let status = cmd.status().expect("failed to spawn cargo");
    if !status.success() {
        eprintln!("failed: {description}");
        process::exit(status.code().unwrap_or(1));
    }
}

fn build_apps(spec: &ArchSpec) {
    let mut cmd = cargo();
    cmd.arg("build").arg("--release");
    for app in USER_APPS {
        cmd.arg("-p").arg(app);
    }
    cmd.arg("-p").arg("shell");
    cmd.arg("--target").arg(spec.user_target_spec);
    cmd.args(UNSTABLE);
    run(&mut cmd, &format!("build user-space ELF apps for {}", spec.name));
}

fn build_kernel(debug: bool, spec: &ArchSpec) {
    let mut cmd = cargo();
    cmd.arg("build");
    if !debug {
        cmd.arg("--release");
    }
    cmd.arg("-p").arg(spec.kernel_package);
    cmd.arg("--target").arg(spec.kernel_target_spec);
    cmd.args(UNSTABLE);
    run(&mut cmd, &format!("build {} kernel", spec.name));
}

fn build_image(debug: bool, launch_qemu: bool, spec: &ArchSpec) {
    let profile = if debug { "debug" } else { "release" };
    let kernel = spec.kernel_elf(profile);
    let mut cmd = cargo();
    cmd.arg("run")
        .arg("--manifest-path")
        .arg("arch/x86_64-runner/Cargo.toml")
        .arg("--")
        .arg(&kernel)
        .arg("x86_64");
    if !launch_qemu {
        cmd.arg("--no-run");
    }
    run(&mut cmd, "create x86_64 disk image");
}

fn pack_rom(spec: &ArchSpec) {
    let mut files: Vec<(&str, Vec<u8>)> = Vec::new();
    let release_dir = spec.user_release_dir();
    for name in USER_APPS {
        let path = release_dir.join(name);
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("missing user ELF {}; run `cargo xtask apps {}` first", path.display(), spec.name);
            process::exit(1);
        };
        files.push((name, bytes));
    }
    files.sort_by_key(|(name, _)| *name);
    let pairs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (*name, bytes.as_slice()))
        .collect();
    let image = system::rom::encode(&pairs);
    let rom_path = spec.rom_path();
    std::fs::write(&rom_path, image).expect("failed to write ROM image");
    println!("==> packed ROM image {}", rom_path.display());
}

fn create_run_disk(spec: &ArchSpec) -> PathBuf {
    let path = workspace_root()
        .join("target")
        .join(format!("{}-run-disk.img", spec.kernel_dir_name));
    let mut image = vec![0u8; RUN_DISK_BYTES];
    image[MBR_SIGNATURE_OFFSET] = 0x55;
    image[MBR_SIGNATURE_OFFSET + 1] = 0xAA;
    std::fs::write(&path, image).expect("failed to create run disk image");
    path
}

fn launch_multiboot(debug: bool, spec: &ArchSpec) {
    let kernel = spec.kernel_elf(if debug { "debug" } else { "release" });
    let disk = create_run_disk(spec);
    let mut cmd = Command::new(spec.qemu_binary);
    cmd.current_dir(workspace_root())
        .arg("-kernel").arg(&kernel)
        .arg("-drive").arg(format!("if=ide,index=0,media=disk,format=raw,file={}", disk.display()))
        .arg("-debugcon").arg("stdio")
        .arg("-display").arg("none")
        .arg("-no-reboot")
        .arg("-no-shutdown");
    println!("==> booting {} in {}", spec.name, spec.qemu_binary);
    let status = cmd.status().expect("failed to spawn qemu");
    if !status.success() {
        eprintln!("failed: boot {} in {}", spec.name, spec.qemu_binary);
        process::exit(status.code().unwrap_or(1));
    }
}

fn build_grub_image(debug: bool) {
    let mut cmd = Command::new("bash");
    cmd.current_dir(workspace_root())
        .arg("arch/x86_32/build-image.sh")
        .env("PROFILE", if debug { "debug" } else { "release" });
    println!("==> create x86_32 GRUB image");
    let status = cmd.status().expect("failed to spawn build-image.sh");
    if !status.success() {
        eprintln!("failed: create x86_32 GRUB image");
        process::exit(status.code().unwrap_or(1));
    }
}

fn run_tests(skip_integration: bool) {
    build_apps(&X86_64);
    pack_rom(&X86_64);
    let mut cmd = cargo();
    cmd.arg("test").arg("--workspace");
    let description = if skip_integration {
        cmd.arg("--exclude").arg("tests-integration");
        "run unit tests (integration excluded)"
    } else {
        "run all unit tests"
    };
    cmd.arg("--").arg("--test-threads=1");
    run(&mut cmd, description);
}

fn usage() {
    eprintln!(
        "usage: cargo xtask <build [x86_64|x86_32] [--debug] | run [x86_64|x86_32] [--debug] | image [x86_32] [--debug] | apps [x86_64|x86_32] | rom [x86_64|x86_32] | test [--skip-integration]>"
    );
}

fn parse_optional_flag(extras: &[String], flag: &str) -> bool {
    let mut found = false;
    for arg in extras {
        if arg == flag && !found {
            found = true;
        } else {
            eprintln!("unknown flag: {arg}");
            usage();
            process::exit(2);
        }
    }
    found
}

fn split_arch(extras: &[String]) -> (&'static ArchSpec, &[String]) {
    match extras.first() {
        Some(token) if !token.starts_with('-') => match arch::find(token) {
            Some(spec) => (spec, &extras[1..]),
            None => {
                eprintln!("unsupported arch: {token}");
                usage();
                process::exit(2);
            }
        },
        _ => (&X86_64, extras),
    }
}

fn require_no_extras(extras: &[String]) {
    if let Some(arg) = extras.first() {
        eprintln!("unknown flag: {arg}");
        usage();
        process::exit(2);
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let subcommand = args.first().map(String::as_str).unwrap_or("build");
    let extras = args.get(1..).unwrap_or(&[]);
    match subcommand {
        "apps" => {
            let (spec, rest) = split_arch(extras);
            require_no_extras(rest);
            build_apps(spec);
        }
        "build" => {
            let (spec, rest) = split_arch(extras);
            let debug = parse_optional_flag(&rest, "--debug");
            build_apps(spec);
            pack_rom(spec);
            build_kernel(debug, spec);
            match spec.image_kind {
                ImageKind::BiosBoot => build_image(debug, false, spec),
                ImageKind::Multiboot => println!(
                    "==> bootable multiboot artifact: {}",
                    spec.kernel_elf(if debug { "debug" } else { "release" }).display()
                ),
            }
        }
        "run" => {
            let (spec, rest) = split_arch(extras);
            let debug = parse_optional_flag(&rest, "--debug");
            build_apps(spec);
            pack_rom(spec);
            build_kernel(debug, spec);
            match spec.image_kind {
                ImageKind::BiosBoot => build_image(debug, true, spec),
                ImageKind::Multiboot => launch_multiboot(debug, spec),
            }
        }
        "image" => {
            let (spec, rest) = split_arch(extras);
            let debug = parse_optional_flag(&rest, "--debug");
            match spec.image_kind {
                ImageKind::BiosBoot => {
                    println!("==> x86_64 disk image is produced by `cargo xtask build`");
                }
                ImageKind::Multiboot => {
                    build_apps(spec);
                    pack_rom(spec);
                    build_kernel(debug, spec);
                    build_grub_image(debug);
                }
            }
        }
        "rom" => {
            let (spec, rest) = split_arch(extras);
            require_no_extras(rest);
            pack_rom(spec);
        }
        "test" => {
            let skip_integration = parse_optional_flag(extras, "--skip-integration");
            run_tests(skip_integration);
        }
        "help" | "--help" | "-h" => usage(),
        other => {
            eprintln!("unknown subcommand: {other}");
            usage();
            process::exit(2);
        }
    }
}
