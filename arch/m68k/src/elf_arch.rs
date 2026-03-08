use kernel::elf::ElfArch;

pub struct M68kElfArch;

impl ElfArch for M68kElfArch {
    fn apply_relocation(&self, _base: usize, _offset: usize, _info: u64, _addend: i64) {
        // Placeholder for m68k relocations
    }
}
