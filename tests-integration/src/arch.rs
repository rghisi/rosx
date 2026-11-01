use std::env;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arch {
    X86_64,
    X86_32,
}

impl Arch {
    pub fn current() -> Arch {
        Self::from_selection(env::var("ROSX_ARCH").ok().as_deref())
    }

    fn from_selection(value: Option<&str>) -> Arch {
        match value {
            None | Some("") | Some("x86_64") => Arch::X86_64,
            Some("x86_32") => Arch::X86_32,
            Some(value) => panic!(
                "ROSX_ARCH is set to '{value}'; valid values are: x86_64, x86_32"
            ),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::X86_32 => "x86_32",
        }
    }

    pub fn qemu_binary(self) -> &'static str {
        match self {
            Arch::X86_64 => "qemu-system-x86_64",
            Arch::X86_32 => "qemu-system-i386",
        }
    }

    pub fn kernel_manifest(self) -> &'static str {
        match self {
            Arch::X86_64 => "arch/x86_64/Cargo.toml",
            Arch::X86_32 => "arch/x86_32/Cargo.toml",
        }
    }

    pub fn kernel_target_spec(self) -> &'static str {
        match self {
            Arch::X86_64 => "arch/x86_64/rosx.json",
            Arch::X86_32 => "arch/x86_32/rosx-i686.json",
        }
    }

    pub fn kernel_dir_name(self) -> &'static str {
        match self {
            Arch::X86_64 => "rosx",
            Arch::X86_32 => "rosx-i686",
        }
    }

    pub fn kernel_binary(self) -> &'static str {
        match self {
            Arch::X86_64 => "rosx",
            Arch::X86_32 => "rosx-x86",
        }
    }

    pub fn boots_via_disk_image(self) -> bool {
        self == Arch::X86_64
    }
}

pub fn not_ported_to_x86_32() -> bool {
    if Arch::current() == Arch::X86_32 {
        eprintln!(
            "skipped on x86_32: not in the ported subset yet (see plans/x86_32-parity-plan.md)"
        );
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::Arch;

    #[test]
    fn unset_selection_defaults_to_x86_64() {
        assert_eq!(Arch::from_selection(None), Arch::X86_64);
    }

    #[test]
    fn empty_selection_defaults_to_x86_64() {
        assert_eq!(Arch::from_selection(Some("")), Arch::X86_64);
    }

    #[test]
    fn explicit_arch_names_select_the_matching_platform() {
        assert_eq!(Arch::from_selection(Some("x86_64")), Arch::X86_64);
        assert_eq!(Arch::from_selection(Some("x86_32")), Arch::X86_32);
    }

    #[test]
    #[should_panic(expected = "ROSX_ARCH is set to 'mips'")]
    fn unknown_selection_panics() {
        Arch::from_selection(Some("mips"));
    }

    #[test]
    fn x86_64_boots_from_a_disk_image_and_x86_32_from_a_multiboot_kernel() {
        assert!(Arch::X86_64.boots_via_disk_image());
        assert!(!Arch::X86_32.boots_via_disk_image());
        assert_eq!(Arch::X86_64.qemu_binary(), "qemu-system-x86_64");
        assert_eq!(Arch::X86_32.qemu_binary(), "qemu-system-i386");
    }
}
