use kernel::elf::arch::ElfArch;

const R_386_RELATIVE: u64 = 8;

pub struct X86_32ElfArch;

impl ElfArch for X86_32ElfArch {
    fn apply_relocation(&self, base: usize, offset: usize, info: u64, addend: i64) {
        if info & 0xff == R_386_RELATIVE {
            let patch_addr = (base + offset) as *mut u32;
            let value = (base as i64 + addend) as u32;
            // SAFETY: ELF loader guarantees offset is within the loaded image bounds
            unsafe { core::ptr::write(patch_addr, value) };
        }
    }
}
