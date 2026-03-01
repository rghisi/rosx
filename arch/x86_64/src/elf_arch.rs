use kernel::elf::ElfArch;

const R_X86_64_RELATIVE: u64 = 8;

pub struct X86_64ElfArch;

impl ElfArch for X86_64ElfArch {
    fn apply_relocation(&self, base: usize, offset: usize, info: u64, addend: i64) {
        if info & 0xffffffff == R_X86_64_RELATIVE {
            let patch_addr = (base + offset) as *mut u64;
            let value = (base as i64 + addend) as u64;
            // SAFETY: The ELF loader guarantees that `offset` is within the loaded image
            // and points to a location that the RELA entry intends to patch.
            unsafe {
                core::ptr::write(patch_addr, value);
            }
        }
    }
}
