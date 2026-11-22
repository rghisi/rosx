use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub mod chain;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arch {
    X86_64,
    X86_32,
}

impl Arch {
    pub fn name(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::X86_32 => "x86_32",
        }
    }

    pub fn from_name(value: &str) -> Option<Arch> {
        match value {
            "x86_64" => Some(Arch::X86_64),
            "x86_32" => Some(Arch::X86_32),
            _ => None,
        }
    }

    pub fn qemu_binary(self) -> &'static str {
        match self {
            Arch::X86_64 => "qemu-system-x86_64",
            Arch::X86_32 => "qemu-system-i386",
        }
    }
}

pub const HDD_IMAGE_BYTES: u64 = 50 * 1024 * 1024;
pub const HDD_IMAGE_NAME: &str = "rosx-hdd.img";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Interactive,
    Headless,
}

#[derive(Debug)]
pub struct ImageError(anyhow::Error);

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:#}", self.0)
    }
}

impl std::error::Error for ImageError {}

pub fn boot_disk_image(kernel_elf: &Path) -> Option<PathBuf> {
    let kernel_dir = kernel_elf.parent()?.parent()?;
    let target_dir = kernel_dir.parent()?;
    let dir_name = kernel_dir.file_name()?;
    Some(target_dir.join(format!("{}-boot-disk.img", dir_name.to_string_lossy())))
}

pub fn run_medium(kernel_elf: &Path, arch: Arch) -> Result<PathBuf, ImageError> {
    let disk = boot_disk_image(kernel_elf).ok_or_else(|| {
        ImageError(anyhow::anyhow!(
            "cannot derive the unified boot disk path from {}",
            kernel_elf.display()
        ))
    })?;
    if !disk.is_file() {
        return Err(ImageError(anyhow::anyhow!(
            "the {} boot medium {} does not exist; run `cargo xtask build {}` first",
            arch.name(),
            disk.display(),
            arch.name()
        )));
    }
    Ok(disk)
}

pub fn qemu_command(arch: Arch, disk: &Path, mode: LaunchMode) -> Command {
    let mut command = Command::new(arch.qemu_binary());
    if arch == Arch::X86_32 {
        command.args(["-cpu", "486"]);
    }
    command
        .arg("-drive")
        .arg(format!("if=none,id=rosxboot,format=raw,file={}", disk.display()));
    command
        .arg("-device")
        .arg("ide-hd,drive=rosxboot,bus=ide.0,unit=0,bootindex=1");
    command.arg("-debugcon").arg("stdio").arg("-no-reboot");
    match mode {
        LaunchMode::Interactive => {
            command.arg("-no-shutdown");
            if arch == Arch::X86_64 {
                command.args(["-d", "cpu_reset"]);
            }
        }
        LaunchMode::Headless => {
            command.arg("-display").arg("none");
        }
    }
    command
}

pub fn attach_ne2k_isa(command: &mut Command, multicast: &str) {
    command
        .arg("-netdev")
        .arg(format!("socket,id=rosxnet,mcast={multicast}"));
    command.arg("-device").arg(format!(
        "ne2k_isa,netdev=rosxnet,iobase=0x300,irq=9"
    ));
}

pub fn attach_ne2k_pci(command: &mut Command) {
    command.arg("-netdev").arg("user,id=rosxnet");
    command.arg("-device").arg("ne2k_pci,netdev=rosxnet");
}

pub fn persistent_hdd_image() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    ensure_hdd_image(&PathBuf::from(home).join("tmp"))
}

fn ensure_hdd_image(dir: &Path) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let image = dir.join(HDD_IMAGE_NAME);
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&image)
    {
        Ok(file) => {
            file.set_len(HDD_IMAGE_BYTES).ok()?;
            Some(image)
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Some(image),
        Err(_) => None,
    }
}

fn hdd_drive_arg(image: &Path) -> String {
    format!(
        "if=ide,index=1,media=disk,format=raw,file={}",
        image.display()
    )
}

pub fn attach_persistent_hdd(command: &mut Command) {
    match persistent_hdd_image() {
        Some(image) => {
            command.arg("-drive").arg(hdd_drive_arg(&image));
        }
        None => {
            eprintln!("note: persistent 50MB hard disk not attached (HOME unset or $HOME/tmp unwritable)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_of(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn contains(args: &[String], flag: &str) -> bool {
        args.iter().any(|arg| arg == flag)
    }

    fn contains_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|pair| pair[0] == flag && pair[1] == value)
    }

    #[test]
    fn parses_both_arch_names() {
        assert_eq!(Arch::from_name("x86_64"), Some(Arch::X86_64));
        assert_eq!(Arch::from_name("x86_32"), Some(Arch::X86_32));
        assert_eq!(Arch::X86_64.name(), "x86_64");
        assert_eq!(Arch::X86_32.name(), "x86_32");
    }

    #[test]
    fn rejects_unknown_arch_names() {
        assert_eq!(Arch::from_name("m68k"), None);
        assert_eq!(Arch::from_name(""), None);
    }

    #[test]
    fn run_medium_refuses_a_missing_unified_disk_naming_the_recipe() {
        for (arch, kernel) in [
            (Arch::X86_64, "/build/target/rosx/release/rosx"),
            (Arch::X86_32, "/build/target/rosx-i486/release/rosx-x86_32"),
        ] {
            let missing = run_medium(Path::new(kernel), arch)
                .expect_err("no disk exists under /build");
            let message = missing.to_string();
            assert!(
                message.contains("-boot-disk.img")
                    && message.contains(&format!("cargo xtask build {}", arch.name())),
                "the refusal must name the artifact and the recipe: {message}"
            );
        }
    }

    #[test]
    fn run_medium_runs_both_arches_from_the_unified_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        for (arch, kernel_relative, disk_relative) in [
            (
                Arch::X86_64,
                "target/rosx/release/rosx",
                "target/rosx-boot-disk.img",
            ),
            (
                Arch::X86_32,
                "target/rosx-i486/release/rosx-x86_32",
                "target/rosx-i486-boot-disk.img",
            ),
        ] {
            let kernel = dir.path().join(kernel_relative);
            std::fs::create_dir_all(kernel.parent().unwrap()).expect("kernel tree");
            let disk = dir.path().join(disk_relative);
            std::fs::write(&disk, b"mbr").expect("boot disk");
            assert_eq!(run_medium(&kernel, arch).expect("the disk now exists"), disk);
        }
    }

    #[test]
    fn interactive_x86_64_uses_the_window_and_keeps_reset_diagnostics() {
        let command = qemu_command(
            Arch::X86_64,
            Path::new("/tmp/rosx-boot-disk.img"),
            LaunchMode::Interactive,
        );
        assert_eq!(command.get_program(), "qemu-system-x86_64");
        let args = args_of(&command);
        assert!(contains_pair(
            &args,
            "-drive",
            "if=none,id=rosxboot,format=raw,file=/tmp/rosx-boot-disk.img"
        ));
        assert!(contains_pair(
            &args,
            "-device",
            "ide-hd,drive=rosxboot,bus=ide.0,unit=0,bootindex=1"
        ));
        assert!(contains(&args, "-no-shutdown"));
        assert!(contains(&args, "-d"));
        assert!(!contains(&args, "-display"));
        assert!(!contains(&args, "-cpu"));
    }

    #[test]
    fn interactive_x86_32_uses_the_window_without_reset_diagnostics() {
        let command = qemu_command(
            Arch::X86_32,
            Path::new("/tmp/rosx-i486-boot-disk.img"),
            LaunchMode::Interactive,
        );
        assert_eq!(command.get_program(), "qemu-system-i386");
        let args = args_of(&command);
        assert!(contains_pair(
            &args,
            "-drive",
            "if=none,id=rosxboot,format=raw,file=/tmp/rosx-i486-boot-disk.img"
        ));
        assert!(contains_pair(
            &args,
            "-device",
            "ide-hd,drive=rosxboot,bus=ide.0,unit=0,bootindex=1"
        ));
        assert!(!contains(&args, "-kernel"));
        assert!(contains(&args, "-no-shutdown"));
        assert!(contains_pair(&args, "-cpu", "486"));
        assert!(!contains(&args, "cpu_reset"));
        assert!(!contains(&args, "-display"));
    }

    #[test]
    fn boot_disk_image_lives_in_the_target_directory_beside_the_kernel_tree() {
        let kernel = Path::new("/build/target/rosx/release/rosx");
        assert_eq!(
            boot_disk_image(kernel),
            Some(PathBuf::from("/build/target/rosx-boot-disk.img"))
        );
        let kernel32 = Path::new("/build/target/rosx-i486/release/rosx-x86_32");
        assert_eq!(
            boot_disk_image(kernel32),
            Some(PathBuf::from("/build/target/rosx-i486-boot-disk.img"))
        );
    }

    #[test]
    fn hdd_image_is_created_at_exact_size_and_never_reopened_shrinking() {
        let dir = tempfile::tempdir().expect("tempdir");
        let image = ensure_hdd_image(dir.path()).expect("image must provision");
        assert_eq!(image.file_name().unwrap(), HDD_IMAGE_NAME);
        assert_eq!(std::fs::metadata(&image).unwrap().len(), HDD_IMAGE_BYTES);
        std::fs::write(&image, b"survives").expect("write marker");
        let again = ensure_hdd_image(dir.path()).expect("second call must return same path");
        assert_eq!(again, image);
        assert_eq!(std::fs::read(&image).unwrap(), b"survives");
    }

    #[test]
    fn hdd_drive_arg_pins_the_secondary_ide_address() {
        assert_eq!(
            hdd_drive_arg(Path::new("/home/dev/tmp/rosx-hdd.img")),
            "if=ide,index=1,media=disk,format=raw,file=/home/dev/tmp/rosx-hdd.img"
        );
    }

    #[test]
    fn headless_launches_suppress_the_window_and_parking() {
        for arch in [Arch::X86_64, Arch::X86_32] {
            let args = args_of(&qemu_command(
                arch,
                Path::new("/tmp/boot-disk.img"),
                LaunchMode::Headless,
            ));
            assert!(contains(&args, "-display"), "{arch:?} lost -display");
            assert!(
                !contains(&args, "-no-shutdown"),
                "{arch:?} headless must let triple-faults exit QEMU"
            );
            assert!(contains(&args, "-debugcon"));
            assert!(contains(&args, "-no-reboot"));
            match arch {
                Arch::X86_32 => assert!(contains_pair(&args, "-cpu", "486")),
                Arch::X86_64 => assert!(!contains(&args, "-cpu")),
            }
        }
    }
}
