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

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Elf32Header {
    pub(crate) e_ident: [u8; 16],
    pub(crate) e_type: u16,
    pub(crate) e_machine: u16,
    pub(crate) e_version: u32,
    pub(crate) e_entry: u32,
    pub(crate) e_phoff: u32,
    pub(crate) e_shoff: u32,
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
pub(crate) struct Elf32Phdr {
    pub(crate) p_type: u32,
    pub(crate) p_offset: u32,
    pub(crate) p_vaddr: u32,
    pub(crate) p_paddr: u32,
    pub(crate) p_filesz: u32,
    pub(crate) p_memsz: u32,
    pub(crate) p_flags: u32,
    pub(crate) p_align: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Elf32Dyn {
    pub(crate) d_tag: i32,
    pub(crate) d_val: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Elf32Rel {
    pub(crate) r_offset: u32,
    pub(crate) r_info: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem;

    #[test]
    fn elf32_header_has_correct_size() {
        assert_eq!(mem::size_of::<Elf32Header>(), 52);
    }

    #[test]
    fn elf32_phdr_has_correct_size() {
        assert_eq!(mem::size_of::<Elf32Phdr>(), 32);
    }

    #[test]
    fn elf32_dyn_has_correct_size() {
        assert_eq!(mem::size_of::<Elf32Dyn>(), 8);
    }

    #[test]
    fn elf32_rel_has_correct_size() {
        assert_eq!(mem::size_of::<Elf32Rel>(), 8);
    }

    #[repr(align(4))]
    struct Aligned<const N: usize>([u8; N]);

    #[test]
    fn elf32_header_fields_read_correctly_from_bytes() {
        let mut storage = Aligned([0u8; 52]);
        let bytes = &mut storage.0;
        bytes[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        bytes[4] = 1; // EI_CLASS = 32-bit
        bytes[24..28].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes()); // e_entry
        bytes[28..32].copy_from_slice(&0x34u32.to_le_bytes());         // e_phoff (= 52, header size)
        bytes[44..46].copy_from_slice(&1u16.to_le_bytes());            // e_phnum

        let header = unsafe { &*(bytes.as_ptr() as *const Elf32Header) };
        assert_eq!(&header.e_ident[..4], &[0x7f, b'E', b'L', b'F']);
        assert_eq!(header.e_ident[4], 1);
        assert_eq!(header.e_entry, 0xDEAD_BEEF);
        assert_eq!(header.e_phoff, 0x34);
        assert_eq!(header.e_phnum, 1);
    }

    #[test]
    fn elf32_phdr_fields_read_correctly_from_bytes() {
        let mut storage = Aligned([0u8; 32]);
        let bytes = &mut storage.0;
        bytes[0..4].copy_from_slice(&1u32.to_le_bytes());       // p_type = PT_LOAD
        bytes[4..8].copy_from_slice(&0x1000u32.to_le_bytes());  // p_offset
        bytes[8..12].copy_from_slice(&0x8000u32.to_le_bytes()); // p_vaddr
        bytes[16..20].copy_from_slice(&0x200u32.to_le_bytes()); // p_filesz
        bytes[20..24].copy_from_slice(&0x400u32.to_le_bytes()); // p_memsz

        let phdr = unsafe { &*(bytes.as_ptr() as *const Elf32Phdr) };
        assert_eq!(phdr.p_type, 1);
        assert_eq!(phdr.p_offset, 0x1000);
        assert_eq!(phdr.p_vaddr, 0x8000);
        assert_eq!(phdr.p_filesz, 0x200);
        assert_eq!(phdr.p_memsz, 0x400);
    }

    #[test]
    fn elf32_rel_fields_read_correctly_from_bytes() {
        let mut storage = Aligned([0u8; 8]);
        let bytes = &mut storage.0;
        bytes[0..4].copy_from_slice(&0x1234u32.to_le_bytes()); // r_offset
        bytes[4..8].copy_from_slice(&0x08u32.to_le_bytes());   // r_info (type 8 = R_386_RELATIVE)

        let rel = unsafe { &*(bytes.as_ptr() as *const Elf32Rel) };
        assert_eq!(rel.r_offset, 0x1234);
        assert_eq!(rel.r_info, 8);
    }
}
