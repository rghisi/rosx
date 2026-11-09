use std::path::PathBuf;

use crate::workspace_root;

pub const USER_APPS: [&str; 31] = ["pi", "randomd", "echo", "bulk", "pong", "ping", "mem", "vfsd", "vfs", "snake", "tetris", "conway", "threads", "waitset", "inputd", "ps2d", "stress", "bench", "atad", "blockd", "ata", "format", "mount", "attach", "rm", "cp", "mkdir", "rmdir", "init", "shell", "exiter"];

pub const BOOTSTRAP_APPS: [&str; 8] = ["atad", "blockd", "attach", "mount", "vfsd", "inputd", "ps2d", "shell"];

pub struct ArchSpec {
    pub name: &'static str,
    pub kernel_package: &'static str,
    pub kernel_target_spec: &'static str,
    pub user_target_spec: &'static str,
    pub kernel_dir_name: &'static str,
    pub kernel_binary: &'static str,
    pub user_dir_name: &'static str,
    pub boot: x86_runner::Arch,
    pub fat_boot_disk: bool,
    pub rom_apps: &'static [&'static str],
}

pub static X86_64: ArchSpec = ArchSpec {
    name: "x86_64",
    kernel_package: "rosx",
    kernel_target_spec: "arch/x86_64/rosx.json",
    user_target_spec: "arch/x86_64/rosx-user.json",
    kernel_dir_name: "rosx",
    kernel_binary: "rosx",
    user_dir_name: "rosx-user",
    boot: x86_runner::Arch::X86_64,
    fat_boot_disk: true,
    rom_apps: &BOOTSTRAP_APPS,
};

pub static X86_32: ArchSpec = ArchSpec {
    name: "x86_32",
    kernel_package: "rosx-x86_32",
    kernel_target_spec: "arch/x86_32/rosx-i486.json",
    user_target_spec: "arch/x86_32/rosx-i486-user.json",
    kernel_dir_name: "rosx-i486",
    kernel_binary: "rosx-x86_32",
    user_dir_name: "rosx-i486-user",
    boot: x86_runner::Arch::X86_32,
    fat_boot_disk: true,
    rom_apps: &BOOTSTRAP_APPS,
};

pub static ALL: [&ArchSpec; 2] = [&X86_64, &X86_32];

pub fn find(name: &str) -> Option<&'static ArchSpec> {
    ALL.into_iter().find(|spec| spec.name == name)
}

impl ArchSpec {
    pub fn kernel_elf(&self, profile: &str) -> PathBuf {
        workspace_root()
            .join("target")
            .join(self.kernel_dir_name)
            .join(profile)
            .join(self.kernel_binary)
    }

    pub fn user_release_dir(&self) -> PathBuf {
        workspace_root().join("target").join(self.user_dir_name).join("release")
    }

    pub fn rom_path(&self) -> PathBuf {
        workspace_root()
            .join("target")
            .join(self.user_dir_name)
            .join("rom.img")
    }
}
