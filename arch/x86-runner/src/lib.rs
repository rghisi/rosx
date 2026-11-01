use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

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

#[derive(Debug)]
pub enum BootMedium {
    DiskImage(PathBuf),
    MultibootKernel(PathBuf),
}

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

pub fn run_medium(kernel_elf: &Path, arch: Arch) -> BootMedium {
    match arch {
        Arch::X86_64 => BootMedium::DiskImage(bios_disk_image_path(kernel_elf)),
        Arch::X86_32 => BootMedium::MultibootKernel(kernel_elf.to_path_buf()),
    }
}

pub fn prepare_run_medium(kernel_elf: &Path, arch: Arch) -> Result<BootMedium, ImageError> {
    let medium = run_medium(kernel_elf, arch);
    if let BootMedium::DiskImage(_) = medium {
        create_bios_disk_image(kernel_elf)?;
    }
    Ok(medium)
}

#[derive(Debug)]
pub struct GrubTooling {
    missing: Vec<String>,
    install_hint: &'static str,
}

impl fmt::Display for GrubTooling {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "missing required tools: {}", self.missing.join(", "))?;
        writeln!(f)?;
        write!(f, "  {}", self.install_hint)
    }
}

impl std::error::Error for GrubTooling {}

pub fn create_grub_image(kernel_elf: &Path, out: &Path) -> Result<(), GrubTooling> {
    let probe = |name: &str| on_path(name).is_some();
    let mkrescue = resolve_grub_mkrescue(&probe).ok_or_else(|| missing_tooling(&probe))?;
    if !probe("xorriso") {
        return Err(missing_tooling(&probe));
    }

    let work = tempfile::tempdir().expect("failed to create grub image staging directory");
    std::fs::create_dir_all(work.path().join("boot/grub")).expect("failed to stage boot/grub");
    std::fs::copy(kernel_elf, work.path().join("boot/kernel"))
        .expect("failed to stage kernel into the grub image");
    std::fs::write(work.path().join("boot/grub/grub.cfg"), grub_cfg_contents())
        .expect("failed to write grub.cfg");

    let status = Command::new(mkrescue)
        .arg("-o")
        .arg(out)
        .arg(work.path())
        .status()
        .expect("failed to run grub-mkrescue");
    if !status.success() {
        panic!("grub-mkrescue exited with {:?}", status.code());
    }
    Ok(())
}

fn resolve_grub_mkrescue(probe: &impl Fn(&str) -> bool) -> Option<&'static str> {
    ["grub-mkrescue", "grub2-mkrescue"]
        .into_iter()
        .find(|tool| probe(tool))
}

fn missing_tooling(probe: &impl Fn(&str) -> bool) -> GrubTooling {
    let mut missing = Vec::new();
    if resolve_grub_mkrescue(probe).is_none() {
        missing.push("grub-mkrescue".to_string());
    }
    if !probe("xorriso") {
        missing.push("xorriso".to_string());
    }
    GrubTooling {
        missing,
        install_hint: install_hint(probe),
    }
}

fn install_hint(probe: &impl Fn(&str) -> bool) -> &'static str {
    if probe("apt-get") {
        "sudo apt install grub-pc-bin grub-common xorriso"
    } else if probe("dnf") {
        "sudo dnf install grub2-tools xorriso"
    } else if probe("pacman") {
        "sudo pacman -S grub xorriso"
    } else {
        "Install: grub (grub-pc-bin), xorriso"
    }
}

fn grub_cfg_contents() -> String {
    "set timeout=0\nset default=0\n\nmenuentry \"rosx\" {\n    multiboot /boot/kernel\n    boot\n}\n".to_string()
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

pub fn create_bios_disk_image(kernel_elf: &Path) -> Result<PathBuf, ImageError> {
    let image = bios_disk_image_path(kernel_elf);
    bootloader::BiosBoot::new(kernel_elf)
        .create_disk_image(&image)
        .map_err(ImageError)?;
    Ok(image)
}

pub fn qemu_command(arch: Arch, medium: &BootMedium, mode: LaunchMode) -> Command {
    let mut command = Command::new(arch.qemu_binary());
    match medium {
        BootMedium::DiskImage(image) => {
            command.arg("-drive").arg(format!("format=raw,file={}", image.display()));
        }
        BootMedium::MultibootKernel(kernel) => {
            command.arg("-kernel").arg(kernel);
        }
    }
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

fn bios_disk_image_path(kernel_elf: &Path) -> PathBuf {
    let stem = kernel_elf.file_stem().expect("kernel path has no file stem");
    kernel_elf.with_file_name(format!("{}-{}.img", stem.to_string_lossy(), Arch::X86_64.name()))
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
    fn x86_64_runs_from_a_bios_disk_image_beside_the_kernel() {
        let kernel = Path::new("/build/target/rosx/release/rosx");
        match run_medium(kernel, Arch::X86_64) {
            BootMedium::DiskImage(image) => {
                assert_eq!(image, Path::new("/build/target/rosx/release/rosx-x86_64.img"));
            }
            other => panic!("expected a disk image, got {other:?}"),
        }
    }

    #[test]
    fn x86_32_runs_from_the_multiboot_kernel_itself() {
        let kernel = Path::new("/build/target/rosx-i686/release/rosx-x86_32");
        match run_medium(kernel, Arch::X86_32) {
            BootMedium::MultibootKernel(runs) => assert_eq!(runs, kernel),
            other => panic!("expected the multiboot kernel, got {other:?}"),
        }
    }

    #[test]
    fn interactive_x86_64_uses_the_window_and_keeps_reset_diagnostics() {
        let image = BootMedium::DiskImage(PathBuf::from("/tmp/rosx-x86_64.img"));
        let command = qemu_command(Arch::X86_64, &image, LaunchMode::Interactive);
        assert_eq!(command.get_program(), "qemu-system-x86_64");
        let args = args_of(&command);
        assert!(contains(&args, "-drive"));
        assert!(contains(&args, "-no-shutdown"));
        assert!(contains(&args, "-d"));
        assert!(!contains(&args, "-display"));
    }

    #[test]
    fn interactive_x86_32_uses_the_window_without_reset_diagnostics() {
        let medium = BootMedium::MultibootKernel(PathBuf::from("/tmp/rosx-x86_32"));
        let command = qemu_command(Arch::X86_32, &medium, LaunchMode::Interactive);
        assert_eq!(command.get_program(), "qemu-system-i386");
        let args = args_of(&command);
        assert!(contains(&args, "-kernel"));
        assert!(contains(&args, "-no-shutdown"));
        assert!(!contains(&args, "cpu_reset"));
        assert!(!contains(&args, "-display"));
    }

    #[test]
    fn grub_mkrescue_resolution_prefers_then_falls_back() {
        assert_eq!(
            resolve_grub_mkrescue(&|n| n == "grub-mkrescue"),
            Some("grub-mkrescue")
        );
        assert_eq!(
            resolve_grub_mkrescue(&|n| n == "grub2-mkrescue"),
            Some("grub2-mkrescue")
        );
        assert_eq!(resolve_grub_mkrescue(&|_| false), None);
    }

    #[test]
    fn grub_config_multiboots_the_staged_kernel() {
        assert_eq!(
            grub_cfg_contents(),
            "set timeout=0\nset default=0\n\nmenuentry \"rosx\" {\n    multiboot /boot/kernel\n    boot\n}\n"
        );
    }

    #[test]
    fn missing_grub_tooling_reports_every_gap_with_a_hint() {
        let probe = |name: &str| name == "grub2-mkrescue";
        let error = missing_tooling(&probe);
        assert_eq!(error.missing, vec!["xorriso".to_string()]);
        assert_eq!(
            error.to_string(),
            "missing required tools: xorriso\n\n  Install: grub (grub-pc-bin), xorriso"
        );
    }

    #[test]
    fn headless_launches_suppress_the_window_and_parking() {
        for (arch, medium) in [
            (Arch::X86_64, BootMedium::DiskImage(PathBuf::from("/tmp/a.img"))),
            (
                Arch::X86_32,
                BootMedium::MultibootKernel(PathBuf::from("/tmp/b")),
            ),
        ] {
            let args = args_of(&qemu_command(arch, &medium, LaunchMode::Headless));
            assert!(contains(&args, "-display"), "{arch:?} lost -display");
            assert!(
                !contains(&args, "-no-shutdown"),
                "{arch:?} headless must let triple-faults exit QEMU"
            );
            assert!(contains(&args, "-debugcon"));
            assert!(contains(&args, "-no-reboot"));
        }
    }
}
