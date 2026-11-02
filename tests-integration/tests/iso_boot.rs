use std::time::Duration;
use tests_integration::arch::Arch;
use tests_integration::kernel_build;
use tests_integration::qemu::QemuSession;

#[test]
fn grub_iso_boots_to_shell_prompt() {
    if Arch::current() != Arch::X86_32 {
        eprintln!(
            "skipped on {}: the GRUB ISO boot gate belongs to the x86_32 port",
            Arch::current().name()
        );
        return;
    }

    let staging = tempfile::TempDir::new().expect("failed to create iso staging directory");
    let iso = staging.path().join("rosx-grub.iso");
    if let Err(tools) = x86_runner::create_grub_image(&kernel_build::kernel_elf(), &iso) {
        eprintln!("skipped: cannot build the GRUB ISO:\n{tools}");
        return;
    }

    let session = QemuSession::spawn_cdrom(&iso);
    session.expect_output("ROSE Shell", Duration::from_secs(60));
    session.expect_output("rose>", Duration::from_secs(60));
}
