use rosx_integration::{repo_root, vm::VmRunner};

#[test]
#[ignore = "requires QEMU and RosX kernel build; run with --ignored"]
fn scheduler_sleep_wakes_task_and_returns_prompt() {
    let root = repo_root();
    let runner = VmRunner::new(root);
    let output = runner.check("sleep 1|rose>").expect("failed to run agent");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "agent failed: {} {}", stdout, stderr);
    assert!(stdout.contains("PASS sleep 1|rose>"), "expected PASS, got: {}", stdout);
}

#[test]
#[ignore = "requires QEMU and RosX kernel build; run with --ignored"]
fn scheduler_pi_command_executes_under_scheduler() {
    let root = repo_root();
    let runner = VmRunner::new(root);
    let output = runner.check("pi|3.14159265").expect("failed to run agent");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "agent failed: {} {}", stdout, stderr);
    assert!(stdout.contains("PASS pi|3.14159265"), "expected PASS, got: {}", stdout);
}
