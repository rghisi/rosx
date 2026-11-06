use std::env;
use std::path::PathBuf;
use std::process::{self, Command};

mod arch;
mod check_isa;

use arch::{ArchSpec, X86_64};
use fat_image::{FatDisk, FatKind};
use x86_runner::{BootMedium, LaunchMode};

const USER_APPS: [&str; 27] = ["pi", "randomd", "echo", "bulk", "pong", "ping", "mem", "vfsd", "vfs", "snake", "tetris", "conway", "threads", "waitset", "inputd", "ps2d", "stress", "bench", "atad", "ata", "format", "mount", "attach", "rm", "cp", "mkdir", "rmdir"];

const RUN_DISK_BYTES: usize = 1024 * 1024;
const MBR_SIGNATURE_OFFSET: usize = 510;
const MIB: u64 = 1024 * 1024;
const FAT_DISK_DEFAULT_MIB: u64 = 16;

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

fn prepare_medium(debug: bool, spec: &ArchSpec) -> BootMedium {
    let kernel = spec.kernel_elf(if debug { "debug" } else { "release" });
    let medium = x86_runner::prepare_run_medium(&kernel, spec.boot)
        .expect("failed to prepare boot medium");
    match &medium {
        BootMedium::DiskImage(_) => println!("==> created {} disk image", spec.name),
        BootMedium::MultibootKernel(elf) => {
            println!("==> bootable multiboot artifact: {}", elf.display())
        }
        BootMedium::CdImage(_) => {
            unreachable!("prepare_run_medium only yields a disk image or the multiboot kernel")
        }
    }
    medium
}

fn launch(medium: &BootMedium, spec: &ArchSpec) {
    let mut cmd = x86_runner::qemu_command(spec.boot, medium, LaunchMode::Interactive);
    if let BootMedium::MultibootKernel(_) = medium {
        let disk = create_run_disk(spec);
        cmd.arg("-drive")
            .arg(format!("if=ide,index=0,media=disk,format=raw,file={}", disk.display()));
    }
    x86_runner::attach_persistent_hdd(&mut cmd);
    run(&mut cmd, &format!("boot {} in qemu", spec.name));
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

struct FatDiskOptions {
    fat32: bool,
    size_mib: u64,
    out: Option<PathBuf>,
}

fn parse_fat_disk_options(extras: &[String]) -> FatDiskOptions {
    let mut options = FatDiskOptions { fat32: false, size_mib: FAT_DISK_DEFAULT_MIB, out: None };
    let mut iter = extras.iter();
    while let Some(token) = iter.next() {
        match token.as_str() {
            "--fat32" => options.fat32 = true,
            "--size" => {
                let Some(value) = iter.next() else {
                    eprintln!("--size wants a MiB count, e.g. --size 64");
                    usage();
                    process::exit(2);
                };
                options.size_mib = value.parse().unwrap_or_else(|_| {
                    eprintln!("--size must be an integer MiB count, got '{value}'");
                    process::exit(2);
                });
            }
            "--out" => {
                let Some(path) = iter.next() else {
                    eprintln!("--out wants a file path");
                    usage();
                    process::exit(2);
                };
                options.out = Some(PathBuf::from(path));
            }
            other => {
                eprintln!("unknown flag: {other}");
                usage();
                process::exit(2);
            }
        }
    }
    options
}

fn pack_fat_disk(spec: &ArchSpec, options: &FatDiskOptions) {
    build_apps(spec);
    let (kind, kind_name) = match options.fat32 {
        true => (FatKind::Fat32, "FAT32"),
        false => (FatKind::Fat16, "FAT16"),
    };
    let mut disk = FatDisk::new(kind, options.size_mib * MIB).unwrap_or_else(|error| {
        eprintln!("cannot lay out a {} MiB {kind_name} volume: {error}", options.size_mib);
        process::exit(1);
    });
    let release_dir = spec.user_release_dir();
    let mut packed = 0usize;
    let mut payload_bytes = 0usize;
    for name in USER_APPS {
        let path = release_dir.join(name);
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("missing user ELF {}; run `cargo xtask apps {}` first", path.display(), spec.name);
            process::exit(1);
        };
        disk.add_file(name, &bytes).unwrap_or_else(|error| {
            eprintln!("cannot pack {name} into the {kind_name} volume: {error}");
            process::exit(1);
        });
        packed += 1;
        payload_bytes += bytes.len();
    }
    let out = options.out.clone().unwrap_or_else(|| {
        workspace_root().join("target").join(format!("{}-fat-disk.img", spec.kernel_dir_name))
    });
    disk.write_to(&out).unwrap_or_else(|error| {
        eprintln!("cannot write {}: {error}", out.display());
        process::exit(1);
    });
    println!(
        "==> packed {packed} user ELFs ({payload_bytes} bytes) into {kind_name} image {}",
        out.display()
    );
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

fn era_script(name: &str) -> PathBuf {
    workspace_root()
        .join("arch")
        .join("x86-86box")
        .join("scripts")
        .join(name)
}

fn docker_reachable() -> bool {
    let direct = Command::new("docker")
        .args(["info", "-f", "{{.ServerVersion}}"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if direct {
        return true;
    }
    Command::new("sg")
        .args(["docker", "-c", "docker info -f {{.ServerVersion}}"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn require_era_tools() {
    let mut missing: Vec<String> = [
        ("sfdisk", "util-linux"),
        ("mformat", "mtools"),
        ("mmd", "mtools"),
        ("mcopy", "mtools"),
        ("xorriso", "xorriso"),
        ("curl", "curl"),
        ("dpkg-deb", "dpkg"),
    ]
    .into_iter()
    .filter(|(tool, _)| check_isa::on_path(tool).is_none())
    .map(|(tool, package)| format!("{tool} ({package})"))
    .collect();
    if !docker_reachable() {
        missing.push("docker, nor reachable via `sg docker` (grub-shell container)".to_string());
    }
    if !missing.is_empty() {
        eprintln!("era-image needs host tools that are missing:");
        for tool in &missing {
            eprintln!("  {tool}");
        }
        process::exit(1);
    }
}

fn create_era_images(debug: bool, spec: &ArchSpec) {
    let profile = if debug { "debug" } else { "release" };
    let kernel = spec.kernel_elf(profile);
    let stages = workspace_root().join("target").join("grub-legacy").join("stages");
    let iso = kernel.with_file_name(format!("{}-legacy.iso", spec.kernel_binary));
    let hdd = kernel.with_file_name(format!("{}-legacy-hdd.img", spec.kernel_binary));

    let mut fetch = Command::new("sh");
    fetch.arg(era_script("fetch-grub-legacy.sh"));
    run(&mut fetch, "fetch the pinned 486-clean GRUB Legacy stages");

    let mut iso_cmd = Command::new("sh");
    iso_cmd
        .arg(era_script("build-legacy-iso.sh"))
        .arg(&kernel)
        .arg(&stages)
        .arg(&iso);
    run(&mut iso_cmd, "build the GRUB Legacy El Torito era ISO");

    let mut hdd_cmd = Command::new("sh");
    hdd_cmd
        .arg(era_script("build-legacy-hdd.sh"))
        .arg(&kernel)
        .arg(&stages)
        .arg(&hdd);
    run(&mut hdd_cmd, "build the GRUB Legacy FAT16 era HDD image");

    println!("==> era artifacts: {} and {}", iso.display(), hdd.display());
}

fn create_flashable_image(debug: bool, spec: &ArchSpec) {
    let kernel = spec.kernel_elf(if debug { "debug" } else { "release" });
    match spec.boot {
        x86_runner::Arch::X86_64 => {
            let image = x86_runner::create_bios_disk_image(&kernel).unwrap_or_else(|err| {
                eprintln!("error: {err}");
                process::exit(1);
            });
            println!("==> created x86_64 disk image {}", image.display());
        }
        x86_runner::Arch::X86_32 => {
            let image = kernel.with_extension("img");
            x86_runner::create_grub_image(&kernel, &image).unwrap_or_else(|err| {
                eprintln!("error: {err}");
                process::exit(1);
            });
            println!("==> created x86_32 GRUB image {}", image.display());
        }
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

fn check_isa_command(spec: &ArchSpec) {
    if spec.boot != x86_runner::Arch::X86_32 {
        eprintln!("check-isa audits the x86_32 486 baseline; {} carries no such pin", spec.name);
        process::exit(2);
    }
    let mut elfs: Vec<&str> = USER_APPS.to_vec();
    elfs.push("shell");
    match check_isa::run_check_isa(spec, &elfs) {
        Err(message) => {
            eprintln!("error: {message}");
            process::exit(1);
        }
        Ok(failures) => {
            if failures.is_empty() {
                println!("==> check-isa {}: kernel and {} user ELFs are 486-clean", spec.name, elfs.len());
                return;
            }
            eprintln!("non-486 mnemonics found:");
            for failure in &failures {
                eprintln!("  {failure}");
            }
            process::exit(1);
        }
    }
}

fn check_isa_stages_command(stages_dir: &std::path::Path) {
    match check_isa::run_check_stages(stages_dir) {
        Err(message) => {
            eprintln!("error: {message}");
            process::exit(1);
        }
        Ok(failures) => {
            if failures.is_empty() {
                println!(
                    "==> check-isa --stages: {} bootloader stages under {} are 486-clean",
                    check_isa::STAGE_FILES.len(),
                    stages_dir.display()
                );
                return;
            }
            eprintln!("post-486 mnemonics found in bootloader stages (the 0.97-67 floor is the last clean build):");
            for failure in &failures {
                eprintln!("  {failure}");
            }
            process::exit(1);
        }
    }
}

fn parse_stages_option(extras: &[String]) -> Result<Option<std::path::PathBuf>, String> {
    let mut iter = extras.iter();
    match iter.next() {
        None => return Ok(None),
        Some(token) if token == "--stages" => {}
        Some(token) => return Err(format!("unknown flag: {token}")),
    }
    let dir = match iter.next() {
        None => workspace_root().join("target").join("grub-legacy").join("stages"),
        Some(path) => std::path::PathBuf::from(path),
    };
    if let Some(extra) = iter.next() {
        return Err(format!("unknown flag: {extra}"));
    }
    Ok(Some(dir))
}

fn usage() {
    eprintln!(
        "usage: cargo xtask <build [x86_64|x86_32] [--debug] | run [x86_64|x86_32] [--debug] | image [x86_64|x86_32] [--debug] | era-image x86_32 [--debug] | fat-disk [x86_64|x86_32] [--fat32] [--size MiB] [--out path] | apps [x86_64|x86_32] | rom [x86_64|x86_32] | check-isa x86_32 [--stages <dir>] | test [--skip-integration]>"
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
            prepare_medium(debug, spec);
        }
        "run" => {
            let (spec, rest) = split_arch(extras);
            let debug = parse_optional_flag(&rest, "--debug");
            build_apps(spec);
            pack_rom(spec);
            build_kernel(debug, spec);
            let medium = prepare_medium(debug, spec);
            launch(&medium, spec);
        }
        "image" => {
            let (spec, rest) = split_arch(extras);
            let debug = parse_optional_flag(&rest, "--debug");
            build_apps(spec);
            pack_rom(spec);
            build_kernel(debug, spec);
            create_flashable_image(debug, spec);
        }
        "era-image" => {
            let (spec, rest) = split_arch(extras);
            if spec.boot != x86_runner::Arch::X86_32 {
                eprintln!("era images are the x86_32 GRUB Legacy boot media; {} boots differently", spec.name);
                process::exit(2);
            }
            let debug = parse_optional_flag(&rest, "--debug");
            require_era_tools();
            build_apps(spec);
            pack_rom(spec);
            build_kernel(debug, spec);
            create_era_images(debug, spec);
        }
        "rom" => {
            let (spec, rest) = split_arch(extras);
            require_no_extras(rest);
            pack_rom(spec);
        }
        "fat-disk" => {
            let (spec, rest) = split_arch(extras);
            let options = parse_fat_disk_options(rest);
            pack_fat_disk(spec, &options);
        }
        "check-isa" => {
            let (spec, rest) = split_arch(extras);
            match parse_stages_option(rest) {
                Ok(Some(dir)) => check_isa_stages_command(&dir),
                Ok(None) => check_isa_command(spec),
                Err(message) => {
                    eprintln!("{message}");
                    usage();
                    process::exit(2);
                }
            }
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
