#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Elf64Header {
    pub(crate) e_ident: [u8; 16],
    pub(crate) e_type: u16,
    pub(crate) e_machine: u16,
    pub(crate) e_version: u32,
    pub(crate) e_entry: u64,
    pub(crate) e_phoff: u64,
    pub(crate) e_shoff: u64,
    pub(crate) e_flags: u32,
    pub(crate) e_ehsize: u16,
    pub(crate) e_phentsize: u16,
    pub(crate) e_phnum: u16,
    pub(crate) e_shentsize: u16,
    pub(crate) e_shnum: u16,
    pub(crate) e_shstrndx: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Elf64Phdr {
    pub(crate) p_type: u32,
    pub(crate) p_flags: u32,
    pub(crate) p_offset: u64,
    pub(crate) p_vaddr: u64,
    pub(crate) p_paddr: u64,
    pub(crate) p_filesz: u64,
    pub(crate) p_memsz: u64,
    pub(crate) p_align: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Elf64Dyn {
    pub(crate) d_tag: i64,
    pub(crate) d_val: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Elf64Rela {
    pub(crate) r_offset: u64,
    pub(crate) r_info: u64,
    pub(crate) r_addend: i64,
}

