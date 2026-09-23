use std::path::PathBuf;
use std::process::Command;

pub struct VmRunner {
    repo_root: PathBuf,
}

impl VmRunner {
    pub fn new(repo_root: PathBuf) -> Self {
        Self { repo_root }
    }

    pub fn check(&self, spec: &str) -> std::io::Result<std::process::Output> {
        let agent = self.repo_root.join("tools/rosx_qemu_agent.py");
        Command::new("python3")
            .arg(agent)
            .arg("--check")
            .arg(spec)
            .current_dir(&self.repo_root)
            .output()
    }
}
