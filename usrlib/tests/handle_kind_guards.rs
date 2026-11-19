use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

const KIND_CONFUSION: &str = "pub fn kind_confusion(c: usrlib::handle::UserConnectionHandle) -> usrlib::handle::UserFutureHandle { c }";
const WRONG_API: &str = "pub fn wrong_api(b: usrlib::handle::UserBindingHandle) { let _ = usrlib::ipc::ipc_send_value(b, 7); }";
const POSITIVE_CONTROL: &str = "pub fn control(c: usrlib::handle::UserConnectionHandle) -> usrlib::handle::UserConnectionHandle { c }";

fn usrlib_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn scratch_root() -> PathBuf {
    usrlib_directory().parent().unwrap().join("target").join("handle-kind-guards")
}

fn check_snippet(name: &str, snippet: &str) -> Output {
    let crate_directory = scratch_root().join(name);
    fs::create_dir_all(crate_directory.join("src")).unwrap();
    fs::write(
        crate_directory.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\nusrlib = {{ path = {:?} }}\n\n[workspace]\n",
            usrlib_directory()
        ),
    )
    .unwrap();
    fs::write(crate_directory.join("src").join("lib.rs"), format!("{snippet}\n")).unwrap();
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .arg("check")
        .arg("--offline")
        .current_dir(&crate_directory)
        .env("CARGO_TARGET_DIR", scratch_root().join("target"))
        .output()
        .expect("failed to spawn cargo check")
}

fn assert_rejected_as_type_mismatch(name: &str, snippet: &str) {
    let output = check_snippet(name, snippet);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{name} compiled but must be rejected:\n{stderr}");
    assert!(stderr.contains("error[E0308]"), "{name} failed, but not with E0308:\n{stderr}");
}

#[test]
fn a_connection_handle_cannot_stand_in_for_a_future_handle() {
    assert_rejected_as_type_mismatch("kind_confusion", KIND_CONFUSION);
}

#[test]
fn a_binding_handle_cannot_be_sent_as_a_connection() {
    assert_rejected_as_type_mismatch("wrong_api", WRONG_API);
}

#[test]
fn the_positive_control_compiles() {
    let output = check_snippet("positive_control", POSITIVE_CONTROL);
    assert!(
        output.status.success(),
        "positive control must compile, so the negative probes are meaningful:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
