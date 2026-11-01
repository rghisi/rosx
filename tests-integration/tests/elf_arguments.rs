use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const ARGUMENTS_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn elf_app_receives_arguments_from_kernel() {
    if tests_integration::arch::not_ported_to_x86_32() {
        return;
    }
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);
    let mark = session.mark();
    session.send_line("pi");
    session.expect_output_since(mark, "args(1): [\"pi\"]", ARGUMENTS_TIMEOUT);
}
