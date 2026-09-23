use rosx_integration::{repo_root, vm::VmRunner};

#[test]
#[ignore = "requires QEMU and RosX kernel build; run with --ignored"]
fn pi_command_is_executed_and_output_contains_digits() {
    let root = repo_root();
    let runner = VmRunner::new(root);
    let output = runner.check("pi|3.14159265").expect("failed to run agent");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "agent failed: {} {}", stdout, stderr);
    assert!(stdout.contains("PASS pi|3.14159265"), "expected PASS, got: {}", stdout);
}

#[test]
fn harness_files_exist() {
    let root = repo_root();
    assert!(root.join("tools/rosx_qemu_agent.py").exists());
    assert!(root.join("tests/shell_checks.txt").exists());
}
