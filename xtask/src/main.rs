use std::env;
use std::path::PathBuf;
use std::process::{self, Command};

const USER_APPS: [&str; 5] = ["hello_elf", "random_gen_server", "snake", "tetris", "conway"];

const UNSTABLE: [&str; 3] = [
    "-Zbuild-std=core,alloc,compiler_builtins",
    "-Zbuild-std-features=compiler-builtins-mem",
    "-Zjson-target-spec",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn cargo() -> Command {
    let mut cmd = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(workspace_root());
    cmd
}

fn run(cmd: &mut Command, description: &str) {
    println!("==> {description}");
    let status = cmd.status().expect("failed to spawn cargo");
    if !status.success() {
        eprintln!("failed: {description}");
        process::exit(status.code().unwrap_or(1));
    }
}

fn build_apps() {
    let mut cmd = cargo();
    cmd.arg("build").arg("--release");
    for app in USER_APPS {
        cmd.arg("-p").arg(app);
    }
    cmd.arg("--target").arg("arch/x86_64/rosx-user.json");
    cmd.args(UNSTABLE);
    run(&mut cmd, "build user-space ELF apps");
}

fn build_kernel(debug: bool) {
    let mut cmd = cargo();
    cmd.arg("build");
    if !debug {
        cmd.arg("--release");
    }
    cmd.arg("-p").arg("rosx");
    cmd.arg("--target").arg("arch/x86_64/rosx.json");
    cmd.args(UNSTABLE);
    run(&mut cmd, "build x86_64 kernel");
}

fn build_image(debug: bool) {
    let profile = if debug { "debug" } else { "release" };
    let kernel = workspace_root()
        .join("target")
        .join("rosx")
        .join(profile)
        .join("rosx");
    let mut cmd = cargo();
    cmd.arg("run")
        .arg("--manifest-path")
        .arg("arch/x86_64-runner/Cargo.toml")
        .arg("--")
        .arg(&kernel)
        .arg("x86_64")
        .arg("--no-run");
    run(&mut cmd, "create x86_64 disk image");
}

fn run_tests(integration: bool) {
    let mut cmd = cargo();
    cmd.arg("test")
        .arg("--workspace")
        .arg("--")
        .arg("--test-threads=1");
    run(&mut cmd, "run all unit tests");
    if integration {
        eprintln!("warning: integration tests are not implemented yet; skipping");
    }
}

fn usage() {
    eprintln!("usage: cargo xtask <build [--debug] | apps | test [--integration]>");
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let subcommand = args.first().map(String::as_str).unwrap_or("build");
    let debug = args.iter().any(|a| a == "--debug");
    match subcommand {
        "apps" => build_apps(),
        "build" => {
            build_apps();
            build_kernel(debug);
            build_image(debug);
        }
        "test" => run_tests(args.iter().any(|a| a == "--integration")),
        "help" | "--help" | "-h" => usage(),
        other => {
            eprintln!("unknown subcommand: {other}");
            usage();
            process::exit(2);
        }
    }
}
