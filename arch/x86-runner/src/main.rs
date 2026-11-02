use std::path::PathBuf;

fn main() {
    let mut args = std::env::args();
    args.next();
    let kernel_binary = PathBuf::from(args.next().expect("expected kernel binary path"));
    let arch = x86_runner::Arch::from_name(&args.next().expect("expected architecture name"))
        .expect("unsupported architecture");
    let no_run = args.any(|a| a == "--no-run");

    let medium = x86_runner::prepare_run_medium(&kernel_binary, arch)
        .expect("failed to prepare boot medium");

    if !no_run {
        let status =
            x86_runner::qemu_command(arch, &medium, x86_runner::LaunchMode::Interactive)
                .status()
                .expect("failed to run QEMU");
        std::process::exit(status.code().unwrap_or(1));
    }
}
