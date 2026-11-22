use std::env;
use std::path::PathBuf;
use std::process::{self, Command};

mod arch;
mod boot_disk;
mod boot_image;
mod boot_test;
mod check_arch;
mod check_isa;
mod check_style;
mod json;
mod records;
mod rust_scan;

use arch::{ArchSpec, X86_64, USER_APPS};
use x86_runner::{BootMedium, LaunchMode};

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
    let medium =
        x86_runner::prepare_run_medium(&kernel, spec.boot).expect("failed to prepare boot medium");
    match &medium {
        BootMedium::DiskImage(_) => println!("==> created {} disk image", spec.name),
        BootMedium::BootDisk(disk) => {
            println!("==> boot medium: the unified disk {}", disk.display())
        }
    }
    medium
}

fn launch(medium: &BootMedium, spec: &ArchSpec) {
    let mut cmd = x86_runner::qemu_command(spec.boot, medium, LaunchMode::Interactive);
    if spec.fat_boot_disk && !matches!(medium, BootMedium::BootDisk(_)) {
        let disk = boot_disk::boot_disk_path(spec);
        cmd.arg("-drive")
            .arg(format!("if=ide,index=0,media=disk,format=raw,file={}", disk.display()));
    }
    x86_runner::attach_persistent_hdd(&mut cmd);
    x86_runner::attach_ne2k_pci(&mut cmd);
    run(&mut cmd, &format!("boot {} in qemu", spec.name));
}

fn pack_rom(spec: &ArchSpec) {
    let mut files: Vec<(&str, Vec<u8>)> = Vec::new();
    let release_dir = spec.user_release_dir();
    for name in spec.rom_apps {
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

fn create_boot_disk(debug: bool, spec: &ArchSpec) {
    if spec.fat_boot_disk {
        boot_disk::create_boot_disk(debug, spec);
    }
}

fn create_flashable_image(debug: bool, spec: &ArchSpec) {
    let image = boot_disk::create_boot_disk(debug, spec);
    println!("==> created the {} unified boot disk {}", spec.name, image.display());
}

fn integration_arch() -> &'static ArchSpec {
    match env::var("ROSX_ARCH") {
        Ok(name) => arch::find(&name).unwrap_or_else(|| {
            eprintln!("unsupported ROSX_ARCH: {name}");
            process::exit(2);
        }),
        Err(_) => &X86_64,
    }
}

fn run_tests(arch: &ArchSpec, skip_integration: bool) {
    build_apps(arch);
    pack_rom(arch);
    let mut cmd = cargo();
    cmd.env("ROSX_ARCH", arch.name);
    if !skip_integration && env::var_os("ROSX_KERNEL_ELF").is_none_or(|value| value.is_empty()) {
        build_kernel(false, arch);
        cmd.env("ROSX_KERNEL_ELF", arch.kernel_elf("release"));
    }
    if !skip_integration && arch.boot == x86_runner::Arch::X86_32 {
        boot_image::stage_chain_artifacts(boot_image::Stage2Flavor::for_arch(arch.boot));
    }
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

fn check_arch_command() {
    match check_arch::run(&workspace_root()) {
        Err(message) => {
            eprintln!("error: {message}");
            process::exit(1);
        }
        Ok(violations) if violations.is_empty() => println!("==> check-arch: dependency arrows and 486 feature floor hold"),
        Ok(violations) => {
            eprintln!("check-arch violations:");
            for violation in &violations {
                eprintln!("  {violation}");
            }
            process::exit(1);
        }
    }
}

fn check_style_command(extras: &[String]) {
    let mut update = false;
    let mut allow_increase: Option<String> = None;
    let mut iter = extras.iter();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--update-baseline" => update = true,
            "--allow-increase" => allow_increase = iter.next().cloned(),
            other => {
                eprintln!("unknown flag: {other}");
                usage();
                process::exit(2);
            }
        }
    }
    if update {
        match check_style::update_baseline(&workspace_root(), allow_increase.as_deref()) {
            Ok(messages) => messages.iter().for_each(|message| println!("==> {message}")),
            Err(message) => {
                eprintln!("error: {message}");
                process::exit(1);
            }
        }
        return;
    }
    match check_style::run(&workspace_root()) {
        Err(message) => {
            eprintln!("error: {message}");
            process::exit(1);
        }
        Ok(violations) if violations.is_empty() => println!("==> check-style: comments, #[allow], unsafe, 32-bit hazards and records hold"),
        Ok(violations) => {
            eprintln!("check-style violations:");
            for violation in &violations {
                eprintln!("  {violation}");
            }
            process::exit(1);
        }
    }
}

fn install_hooks_command() {
    let mut cmd = Command::new("git");
    cmd.args(["config", "core.hooksPath", ".githooks"]).current_dir(workspace_root());
    run(&mut cmd, "point core.hooksPath at .githooks");
}

fn decisions_command() {
    match records::list_open(&workspace_root().join("docs")) {
        Ok(lines) => lines.iter().for_each(|line| println!("{line}")),
        Err(message) => {
            eprintln!("error: {message}");
            process::exit(1);
        }
    }
}

fn gate_stages() {
    println!("==> check stage: check-style");
    check_style_command(&[]);
    println!("==> check stage: check-arch");
    check_arch_command();
    println!("==> check stage: check-isa");
    let spec = arch::find("x86_32").expect("x86_32 is a registered arch");
    build_apps(spec);
    pack_rom(spec);
    build_kernel(false, spec);
    check_isa_command(spec);
}

fn check_command() {
    gate_stages();
    println!("==> check stage: tests");
    run_tests(&X86_64, true);
    println!("==> check: all stages green");
}

fn verify_command() {
    if env::var_os("ROSX_KERNEL_ELF").is_some_and(|value| !value.is_empty()) {
        eprintln!(
            "verify builds each arch's kernel itself; unset ROSX_KERNEL_ELF (it exists for the by-hand cargo test flow)"
        );
        process::exit(2);
    }
    gate_stages();
    for name in ["x86_64", "x86_32"] {
        let spec = arch::find(name).expect("verify archs are registered");
        println!("==> verify stage: build {}", spec.name);
        build_apps(spec);
        pack_rom(spec);
        build_kernel(false, spec);
        create_boot_disk(false, spec);
        prepare_medium(false, spec);
        println!("==> verify stage: integration {}", spec.name);
        run_tests(spec, false);
    }
    println!("==> verify stage: boot-test");
    boot_test::run_boot_test();
    println!("==> verify: gate, build, integration on every arch, and boot-test all green");
}

fn check_isa_command(spec: &ArchSpec) {
    if spec.boot != x86_runner::Arch::X86_32 {
        eprintln!("check-isa audits the x86_32 486 baseline; {} carries no such pin", spec.name);
        process::exit(2);
    }
    let elfs: Vec<&str> = USER_APPS.to_vec();
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

fn check_isa_bootloader_command() {
    match check_isa::run_check_isa_bootloader() {
        Err(message) => {
            eprintln!("error: {message}");
            process::exit(1);
        }
        Ok(failures) => {
            if failures.is_empty() {
                println!(
                    "==> check-isa bootloader: {} bootloader ELFs are 486-clean",
                    check_isa::BOOTLOADER_ELF_NAMES.len()
                );
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

fn usage() {
    eprintln!(
        "usage: cargo xtask <build [x86_64|x86_32] [--debug] | run [x86_64|x86_32] [--debug] | image [x86_64|x86_32] [--debug] | apps [x86_64|x86_32] | rom [x86_64|x86_32] | check-isa x86_32 | check-isa bootloader | check-arch | check-style [--update-baseline [--allow-increase <reason>]] | decisions | install-hooks | check | verify | test [--skip-integration] | boot-image [32|64|floppy] | boot-test | boot-artifacts [x86_64|x86_32]>"
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
            create_boot_disk(debug, spec);
            prepare_medium(debug, spec);
        }
        "run" => {
            let (spec, rest) = split_arch(extras);
            let debug = parse_optional_flag(&rest, "--debug");
            build_apps(spec);
            pack_rom(spec);
            build_kernel(debug, spec);
            create_boot_disk(debug, spec);
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
        "rom" => {
            let (spec, rest) = split_arch(extras);
            require_no_extras(rest);
            pack_rom(spec);
        }
        "check-isa" => {
            if extras.first().map(String::as_str) == Some("bootloader") {
                check_isa_bootloader_command();
            } else {
                let (spec, rest) = split_arch(extras);
                require_no_extras(rest);
                check_isa_command(spec);
            }
        }
        "test" => {
            let skip_integration = parse_optional_flag(extras, "--skip-integration");
            run_tests(&integration_arch(), skip_integration);
            if !skip_integration {
                boot_test::run_boot_test();
            }
        }
        "check-style" => check_style_command(extras),
        "install-hooks" => {
            require_no_extras(extras);
            install_hooks_command();
        }
        "decisions" => {
            require_no_extras(extras);
            decisions_command();
        }
        "check-arch" => {
            require_no_extras(extras);
            check_arch_command();
        }
        "check" => {
            require_no_extras(extras);
            check_command();
        }
        "verify" => {
            require_no_extras(extras);
            verify_command();
        }
        "boot-image" => {
            if extras.len() != 1 {
                eprintln!("boot-image expects exactly one of: 32, 64, floppy");
                usage();
                process::exit(2);
            }
            boot_image::boot_image_command(&extras[0]);
        }
        "boot-test" => {
            require_no_extras(extras);
            boot_test::run_boot_test();
        }
        "boot-artifacts" => {
            let (spec, rest) = split_arch(extras);
            require_no_extras(rest);
            boot_image::stage_chain_artifacts(boot_image::Stage2Flavor::for_arch(spec.boot));
        }
        "help" | "--help" | "-h" => usage(),
        other => {
            eprintln!("unknown subcommand: {other}");
            usage();
            process::exit(2);
        }
    }
}
