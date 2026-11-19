use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_TIMEOUT: Duration = Duration::from_secs(90);
const LEAK_TIMEOUT: Duration = Duration::from_secs(30);
const REPORT_TIMEOUT: Duration = Duration::from_secs(15);

fn abandoned_count(line: &str) -> usize {
    line.split_whitespace()
        .nth(2)
        .and_then(|count| count.parse().ok())
        .unwrap_or_else(|| panic!("no abandoned count in {line:?}"))
}

#[test]
fn a_dying_group_forfeits_its_abandoned_futures_and_the_pool_stays_usable() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_TIMEOUT);

    let first_run = session.mark();
    session.send_line("leaky");
    session.expect_output_since(first_run, "LEAKY ABANDONED ", LEAK_TIMEOUT);
    session.expect_output_since(first_run, "rose>", LEAK_TIMEOUT);
    let first = abandoned_count(
        session
            .content_since(first_run)
            .lines()
            .find(|line| line.contains("LEAKY ABANDONED "))
            .unwrap(),
    );
    assert!(first > 512, "leaky must have filled most of the pool, got {first}");

    let second_run = session.mark();
    session.send_line("leaky");
    session.expect_output_since(second_run, "LEAKY ABANDONED ", LEAK_TIMEOUT);
    session.expect_output_since(second_run, "rose>", LEAK_TIMEOUT);
    let second = abandoned_count(
        session
            .content_since(second_run)
            .lines()
            .find(|line| line.contains("LEAKY ABANDONED "))
            .unwrap(),
    );
    assert!(
        second + 16 >= first,
        "the dead group must have returned its slots: first abandoned {first}, second only {second}"
    );

    let after_report = session.mark();
    session.send_line("exiter 7");
    session.expect_output_since(after_report, "exiter exited with code 7", REPORT_TIMEOUT);
    session.expect_output_since(after_report, "rose>", REPORT_TIMEOUT);
}
