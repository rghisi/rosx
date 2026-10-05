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

fn build_image(debug: bool, launch_qemu: bool) {
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
        .arg("x86_64");
    if !launch_qemu {
        cmd.arg("--no-run");
    }
    run(&mut cmd, "create x86_64 disk image");
}

fn run_tests(skip_integration: bool) {
    build_apps();
    let mut cmd = cargo();
    cmd.arg("test").arg("--workspace");
    let description = if skip_integration {
        cmd.arg("--exclude").arg("tests-integration");
        "run unit tests (integration excluded)"
    } else {
        "run all unit tests"
    };
    cmd.arg("--").arg("--test-threads=1");
    run(&mut cmd, description);
}

fn usage() {
    eprintln!(
        "usage: cargo xtask <build [--debug] | run [x86_64] [--debug] | apps | test [--skip-integration]>"
    );
}

fn parse_optional_flag(extras: &[String], flag: &str) -> bool {
    let mut found = false;
    for arg in extras {
        if arg == flag && !found {
            found = true;
        } else {
            eprintln!("unknown flag: {arg}");
            usage();
            process::exit(2);
        }
    }
    found
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let subcommand = args.first().map(String::as_str).unwrap_or("build");
    let extras = args.get(1..).unwrap_or(&[]);
    match subcommand {
        "apps" => {
            if let Some(arg) = extras.first() {
                eprintln!("unknown flag: {arg}");
                usage();
                process::exit(2);
            }
            build_apps();
        }
        "build" => {
            let debug = parse_optional_flag(extras, "--debug");
            build_apps();
            build_kernel(debug);
            build_image(debug, false);
        }
        "run" => {
            let extras = match args.get(1) {
                Some(arch) if !arch.starts_with('-') => {
                    if arch != "x86_64" {
                        eprintln!("unsupported arch: {arch}");
                        usage();
                        process::exit(2);
                    }
                    &args[2..]
                }
                _ => extras,
            };
            let debug = parse_optional_flag(extras, "--debug");
            build_apps();
            build_kernel(debug);
            build_image(debug, true);
        }
        "test" => {
            let skip_integration = parse_optional_flag(extras, "--skip-integration");
            run_tests(skip_integration);
        }
        "help" | "--help" | "-h" => usage(),
        other => {
            eprintln!("unknown subcommand: {other}");
            usage();
            process::exit(2);
        }
    }
}
