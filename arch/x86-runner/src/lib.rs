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
    MultibootKernel { kernel: PathBuf, rom_image: PathBuf },
    CdImage(PathBuf),
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

pub fn rom_image_for(kernel_elf: &Path, arch: Arch) -> Result<PathBuf, ImageError> {
    resolve_rom_image(std::env::var_os("ROSX_ROM_IMAGE"), kernel_elf, arch)
}

fn resolve_rom_image(
    override_env: Option<std::ffi::OsString>,
    kernel_elf: &Path,
    arch: Arch,
) -> Result<PathBuf, ImageError> {
    if let Some(from_env) = override_env {
        let path = PathBuf::from(&from_env);
        if path.is_file() {
            return Ok(path);
        }
        return Err(ImageError(anyhow::anyhow!(
            "ROSX_ROM_IMAGE '{}' is not a file",
            path.display()
        )));
    }
    let user_dir = match arch {
        Arch::X86_64 => "rosx-user",
        Arch::X86_32 => "rosx-i486-user",
    };
    let derived = kernel_elf
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("kernel path {} is too shallow to locate the ROM beside it", kernel_elf.display()))
        .join(user_dir)
        .join("rom.img");
    if derived.is_file() {
        return Ok(derived);
    }
    Err(ImageError(anyhow::anyhow!(
        "missing ROM image {}; run `cargo xtask rom {}` first",
        derived.display(),
        arch.name()
    )))
}

pub fn prepare_run_medium(kernel_elf: &Path, arch: Arch) -> Result<BootMedium, ImageError> {
    match arch {
        Arch::X86_64 => {
            create_bios_disk_image(kernel_elf)?;
            Ok(BootMedium::DiskImage(bios_disk_image_path(kernel_elf)))
        }
        Arch::X86_32 => Ok(BootMedium::MultibootKernel {
            kernel: kernel_elf.to_path_buf(),
            rom_image: rom_image_for(kernel_elf, arch)?,
        }),
    }
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

pub fn create_grub_image(kernel_elf: &Path, rom_image: &Path, out: &Path) -> Result<(), GrubTooling> {
    let probe = |name: &str| on_path(name).is_some();
    let mkrescue = resolve_grub_mkrescue(&probe).ok_or_else(|| missing_tooling(&probe))?;
    if !probe("xorriso") || !probe("mformat") {
        return Err(missing_tooling(&probe));
    }

    let work = tempfile::tempdir().expect("failed to create grub image staging directory");
    std::fs::create_dir_all(work.path().join("boot/grub")).expect("failed to stage boot/grub");
    std::fs::copy(kernel_elf, work.path().join("boot/kernel"))
        .expect("failed to stage kernel into the grub image");
    std::fs::copy(rom_image, work.path().join("boot/rom"))
        .expect("failed to stage the ROM image into the grub image");
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
    if !probe("mformat") {
        missing.push("mformat (mtools)".to_string());
    }
    GrubTooling {
        missing,
        install_hint: install_hint(probe),
    }
}

fn install_hint(probe: &impl Fn(&str) -> bool) -> &'static str {
    if probe("apt-get") {
        "sudo apt install grub-pc-bin grub-common xorriso mtools"
    } else if probe("dnf") {
        "sudo dnf install grub2-tools xorriso mtools"
    } else if probe("pacman") {
        "sudo pacman -S grub xorriso mtools"
    } else {
        "Install: grub (grub-pc-bin), xorriso, mtools"
    }
}

fn grub_cfg_contents() -> String {
    "set timeout=0\nset default=0\n\nmenuentry \"rosx\" {\n    multiboot /boot/kernel\n    module /boot/rom\n    boot\n}\n".to_string()
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

pub fn create_bios_disk_image(kernel_elf: &Path) -> Result<PathBuf, ImageError> {
    let image = bios_disk_image_path(kernel_elf);
    let rom_image = rom_image_for(kernel_elf, Arch::X86_64)?;
    let mut boot = bootloader::BiosBoot::new(kernel_elf);
    boot.set_ramdisk(&rom_image);
    boot.create_disk_image(&image).map_err(ImageError)?;
    Ok(image)
}

pub fn qemu_command(arch: Arch, medium: &BootMedium, mode: LaunchMode) -> Command {
    let mut command = Command::new(arch.qemu_binary());
    if arch == Arch::X86_32 {
        command.args(["-cpu", "486"]);
    }
    match medium {
        BootMedium::DiskImage(image) => {
            command.arg("-drive").arg(format!("format=raw,file={}", image.display()));
        }
        BootMedium::MultibootKernel { kernel, rom_image } => {
            command.arg("-kernel").arg(kernel);
            command.arg("-initrd").arg(rom_image);
        }
        BootMedium::CdImage(iso) => {
            command.arg("-cdrom").arg(iso);
            command.arg("-boot").arg("order=d");
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
    fn x86_64_runs_from_a_bios_disk_image_beside_the_kernel() {
        let kernel = Path::new("/build/target/rosx/release/rosx");
        assert_eq!(
            bios_disk_image_path(kernel),
            Path::new("/build/target/rosx/release/rosx-x86_64.img")
        );
    }

    #[test]
    fn the_rom_is_found_beside_the_kernel_build_tree() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let kernel = workspace.path().join("target/rosx-i486/release/rosx-x86_32");
        std::fs::create_dir_all(kernel.parent().unwrap()).expect("kernel dir");
        let rom = workspace.path().join("target/rosx-i486-user/rom.img");
        std::fs::create_dir_all(rom.parent().unwrap()).expect("rom dir");
        std::fs::write(&rom, b"rom").expect("rom file");

        let found = resolve_rom_image(None, &kernel, Arch::X86_32).expect("sibling rom must be derived");
        assert_eq!(found, rom);
    }

    #[test]
    fn a_missing_sibling_rom_names_the_path_it_looked_for() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let kernel = workspace.path().join("target/rosx-i486/release/rosx-x86_32");
        let error = resolve_rom_image(None, &kernel, Arch::X86_32)
            .err()
            .expect("a missing rom must be an error");
        assert!(
            error.to_string().contains("rosx-i486-user/rom.img"),
            "the error must name the candidate path: {error}"
        );
    }

    #[test]
    fn the_x86_32_rom_carves_from_the_i486_user_tree() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let kernel = workspace.path().join("target/rosx-i486/release/rosx-x86_32");
        let rom = workspace.path().join("target/rosx-i486-user/rom.img");
        std::fs::create_dir_all(rom.parent().unwrap()).expect("rom dir");
        std::fs::write(&rom, b"rom").expect("rom file");
        match prepare_run_medium(&kernel, Arch::X86_32).expect("the x86_32 medium needs only the rom") {
            BootMedium::MultibootKernel { kernel: k, rom_image } => {
                assert_eq!(k, kernel);
                assert_eq!(rom_image, rom);
            }
            other => panic!("expected the multiboot medium, got {other:?}"),
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
        assert!(!contains(&args, "-cpu"));
    }

    #[test]
    fn interactive_x86_32_uses_the_window_without_reset_diagnostics() {
        let medium = BootMedium::MultibootKernel {
            kernel: PathBuf::from("/tmp/rosx-x86_32"),
            rom_image: PathBuf::from("/tmp/rom.img"),
        };
        let command = qemu_command(Arch::X86_32, &medium, LaunchMode::Interactive);
        assert_eq!(command.get_program(), "qemu-system-i386");
        let args = args_of(&command);
        assert!(contains(&args, "-kernel"));
        assert!(contains_pair(&args, "-initrd", "/tmp/rom.img"));
        assert!(contains(&args, "-no-shutdown"));
        assert!(contains_pair(&args, "-cpu", "486"));
        assert!(!contains(&args, "cpu_reset"));
        assert!(!contains(&args, "-display"));
    }

    #[test]
    fn cd_image_medium_boots_from_the_drive() {
        let medium = BootMedium::CdImage(PathBuf::from("/tmp/rosx-grub.iso"));
        let command = qemu_command(Arch::X86_32, &medium, LaunchMode::Headless);
        assert_eq!(command.get_program(), "qemu-system-i386");
        let args = args_of(&command);
        assert!(contains(&args, "-cdrom"));
        assert!(contains_pair(&args, "-cpu", "486"));
        let boot_at = args.iter().position(|arg| arg == "-boot").expect("missing -boot");
        assert_eq!(args[boot_at + 1], "order=d");
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
    fn grub_config_multiboots_the_staged_kernel_with_the_rom_module() {
        assert_eq!(
            grub_cfg_contents(),
            "set timeout=0\nset default=0\n\nmenuentry \"rosx\" {\n    multiboot /boot/kernel\n    module /boot/rom\n    boot\n}\n"
        );
    }

    #[test]
    fn missing_grub_tooling_reports_every_gap_with_a_hint() {
        let probe = |name: &str| name == "grub2-mkrescue";
        let error = missing_tooling(&probe);
        assert_eq!(error.missing, vec!["xorriso".to_string(), "mformat (mtools)".to_string()]);
        assert_eq!(
            error.to_string(),
            "missing required tools: xorriso, mformat (mtools)\n\n  Install: grub (grub-pc-bin), xorriso, mtools"
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
        for (arch, medium) in [
            (Arch::X86_64, BootMedium::DiskImage(PathBuf::from("/tmp/a.img"))),
            (
                Arch::X86_32,
                BootMedium::MultibootKernel {
                    kernel: PathBuf::from("/tmp/b"),
                    rom_image: PathBuf::from("/tmp/b-rom.img"),
                },
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
            match arch {
                Arch::X86_32 => assert!(contains_pair(&args, "-cpu", "486")),
                Arch::X86_64 => assert!(!contains(&args, "-cpu")),
            }
        }
    }
}
