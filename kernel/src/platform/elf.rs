pub trait ElfArch {
    fn apply_relocation(&self, base: usize, offset: usize, info: u64, addend: i64);
}
