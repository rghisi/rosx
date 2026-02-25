use std::path::PathBuf;
use std::process::Command;

fn main() {
    let mut args = std::env::args();
    args.next();
    let kernel_binary = PathBuf::from(args.next().expect("expected kernel binary path"));
    let arch = args.next().expect("expected architecture name");
    let no_run = args.any(|a| a == "--no-run");

    let stem = kernel_binary.file_stem().unwrap().to_str().unwrap();
    let disk_image = kernel_binary.with_file_name(format!("{}-{}.img", stem, arch));

    bootloader::BiosBoot::new(&kernel_binary)
        .create_disk_image(&disk_image)
        .expect("failed to create BIOS disk image");

    if !no_run {
        let status = Command::new("qemu-system-x86_64")
            .args(["-drive", &format!("format=raw,file={}", disk_image.display())])
            .args(["-debugcon", "stdio"])
            .arg("-no-reboot")
            .status()
            .expect("failed to run QEMU");

        std::process::exit(status.code().unwrap_or(1));
    }
}
