use std::path::PathBuf;
use std::process::Command;

fn main() {
    let mut args = std::env::args();
    args.next();
    let kernel_binary = PathBuf::from(args.next().expect("expected kernel binary path"));
    let arch = args.next().expect("expected architecture name");
    let mut no_run = false;
    let mut qmp_path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-run" => no_run = true,
            "--qmp" => {
                qmp_path = Some(args.next().expect("--qmp requires a path"));
            }
            _ => {}
        }
    }

    let stem = kernel_binary.file_stem().unwrap().to_str().unwrap();
    let disk_image = kernel_binary.with_file_name(format!("{}-{}.img", stem, arch));

    bootloader::BiosBoot::new(&kernel_binary)
        .create_disk_image(&disk_image)
        .expect("failed to create BIOS disk image");

    if !no_run {
        let mut cmd = Command::new("qemu-system-x86_64");
        cmd.args(["-drive", &format!("format=raw,file={}", disk_image.display())])
            .args(["-debugcon", "stdio"])
            .arg("-no-reboot")
            .arg("-no-shutdown")
            .args(["-d", "cpu_reset"]);
        if let Some(ref qmp) = qmp_path {
            cmd.arg("-qmp").arg(format!("unix={},server,nowait", qmp));
            cmd.arg("-display").arg("none");
        }
        let status = cmd.status().expect("failed to run QEMU");
        std::process::exit(status.code().unwrap_or(1));
    }
}
