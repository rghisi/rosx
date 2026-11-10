use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const TABLE_TIMEOUT: Duration = Duration::from_secs(30);

fn run_ps(session: &mut QemuSession) -> String {
    let mark = session.mark();
    session.send_line("ps");
    session.expect_output_since(mark, "processes,", TABLE_TIMEOUT);
    session.content_since(mark)
}

fn process_row(report: &str, name: &str) -> Option<(u32, u64)> {
    for line in report.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() == 5 && fields[4] == name {
            let pid = fields[0].parse::<u32>().ok()?;
            let mem_kib = fields[3].parse::<u64>().ok()?;
            return Some((pid, mem_kib));
        }
    }
    None
}

#[test]
fn ps_reports_a_process_table_over_ipc() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);

    let report = run_ps(&mut session);
    assert!(report.contains("PID") && report.contains("NAME"), "header missing: {report}");

    let after_report = session.mark();
    session.expect_output_since(after_report, "rose>", TABLE_TIMEOUT);
}

#[test]
fn ps_lists_live_processes_by_their_program_names() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);

    let report = run_ps(&mut session);
    assert!(process_row(&report, "ps").is_some(), "ps must list itself: {report}");
    assert!(process_row(&report, "shell").is_some(), "ps must list the shell: {report}");
}

#[test]
fn ps_accounts_each_process_stack_to_its_own_group() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);

    let report = run_ps(&mut session);
    let (ps_pid, ps_mem_kib) = process_row(&report, "ps").expect("ps lists itself");
    let (_, shell_mem_kib) = process_row(&report, "shell").expect("ps lists the shell");

    assert!(ps_pid > 0, "ps carries a real pid");
    assert!(
        ps_mem_kib >= 48,
        "a process must account at least its own stack, got {ps_mem_kib} KiB\n{report}"
    );
    assert!(
        shell_mem_kib >= 48,
        "the shell must account at least its own stack, got {shell_mem_kib} KiB\n{report}"
    );
}
