use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const ARGUMENTS_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn shell_forwards_command_arguments_to_elf_app() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);
    let mark = session.mark();
    session.send_line("pi alpha beta");
    session.expect_output_since(
        mark,
        "args(3): [\"pi\", \"alpha\", \"beta\"]",
        ARGUMENTS_TIMEOUT,
    );
}
