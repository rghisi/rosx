use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arch {
    X86_64,
}

impl Arch {
    pub fn name(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
        }
    }

    pub fn from_name(value: &str) -> Option<Arch> {
        match value {
            "x86_64" => Some(Arch::X86_64),
            _ => None,
        }
    }

    pub fn qemu_binary(self) -> &'static str {
        match self {
            Arch::X86_64 => "qemu-system-x86_64",
        }
    }
}

#[derive(Debug)]
pub enum BootMedium {
    DiskImage(PathBuf),
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
    }
}

pub fn prepare_run_medium(kernel_elf: &Path, arch: Arch) -> Result<BootMedium, ImageError> {
    let medium = run_medium(kernel_elf, arch);
    if let BootMedium::DiskImage(_) = medium {
        create_bios_disk_image(kernel_elf)?;
    }
    Ok(medium)
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
    }
    command.arg("-debugcon").arg("stdio").arg("-no-reboot");
    match mode {
        LaunchMode::Interactive => {
            command.arg("-no-shutdown");
            command.args(["-d", "cpu_reset"]);
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
