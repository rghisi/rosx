use std::path::PathBuf;

use crate::workspace_root;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    BiosBoot,
    Multiboot,
}

pub struct ArchSpec {
    pub name: &'static str,
    pub kernel_package: &'static str,
    pub kernel_target_spec: &'static str,
    pub user_target_spec: &'static str,
    pub kernel_dir_name: &'static str,
    pub kernel_binary: &'static str,
    pub user_dir_name: &'static str,
    pub qemu_binary: &'static str,
    pub image_kind: ImageKind,
}

pub static X86_64: ArchSpec = ArchSpec {
    name: "x86_64",
    kernel_package: "rosx",
    kernel_target_spec: "arch/x86_64/rosx.json",
    user_target_spec: "arch/x86_64/rosx-user.json",
    kernel_dir_name: "rosx",
    kernel_binary: "rosx",
    user_dir_name: "rosx-user",
    qemu_binary: "qemu-system-x86_64",
    image_kind: ImageKind::BiosBoot,
};

pub static X86_32: ArchSpec = ArchSpec {
    name: "x86_32",
    kernel_package: "rosx-x86_32",
    kernel_target_spec: "arch/x86_32/rosx-i686.json",
    user_target_spec: "arch/x86_32/rosx-i686-user.json",
    kernel_dir_name: "rosx-i686",
    kernel_binary: "rosx-x86_32",
    user_dir_name: "rosx-i686-user",
    qemu_binary: "qemu-system-i386",
    image_kind: ImageKind::Multiboot,
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
