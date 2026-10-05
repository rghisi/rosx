use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub fn kernel_elf() -> PathBuf {
    if let Some(from_env) = env::var_os("ROSX_KERNEL_ELF") {
        if !from_env.is_empty() {
            let path = PathBuf::from(&from_env);
            return path.canonicalize().unwrap_or_else(|err| {
                panic!(
                    "ROSX_KERNEL_ELF is set to '{}' but it could not be canonicalized: {err}",
                    path.display()
                )
            });
        }
    }

    static KERNEL_ELF: OnceLock<PathBuf> = OnceLock::new();
    KERNEL_ELF.get_or_init(build_kernel).clone()
}

fn build_kernel() -> PathBuf {
    let profile = env::var("ROSX_QEMU_PROFILE").unwrap_or_else(|_| "dev".to_string());
    let profile_dir = match profile.as_str() {
        "dev" => "debug",
        "release" => "release",
        other => panic!("ROSX_QEMU_PROFILE is set to '{other}'; valid values are: dev, release"),
    };

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let manifest_path = repo_root.join("arch").join("x86_64").join("Cargo.toml");
    let target_spec = repo_root.join("arch").join("x86_64").join("rosx.json");

    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(&cargo);
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--target")
        .arg(&target_spec)
        .arg("-Z")
        .arg("build-std=core,alloc,compiler_builtins")
        .arg("-Z")
        .arg("build-std-features=compiler-builtins-mem")
        .arg("-Z")
        .arg("json-target-spec")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("CARGO_ENCODED_RUSTFLAGS");
    let mut command_line = format!(
        "{} build --manifest-path {} --target {} -Z build-std=core,alloc,compiler_builtins -Z build-std-features=compiler-builtins-mem -Z json-target-spec",
        cargo.to_string_lossy(),
        manifest_path.display(),
        target_spec.display(),
    );
    if profile_dir == "release" {
        command.arg("--release");
        command_line.push_str(" --release");
    }

    let status =
        command.status().unwrap_or_else(|err| panic!("failed to execute `{command_line}`: {err}"));
    if !status.success() {
        panic!(
            "kernel build failed: `{command_line}` exited with code {:?}",
            status.code()
        );
    }

    let artifact = repo_root
        .join("target")
        .join("rosx")
        .join(profile_dir)
        .join("rosx");
    if !artifact.is_file() {
        panic!(
            "kernel ELF not found at '{}'; set ROSX_KERNEL_ELF to a prebuilt kernel ELF",
            artifact.display()
        );
    }
    artifact
}
