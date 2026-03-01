pub mod arch;
pub(crate) mod structs;

use alloc::vec;
use alloc::vec::Vec;
use core::mem;
use structs::{Elf64Dyn, Elf64Header, Elf64Phdr, Elf64Rela};

pub use arch::ElfArch;

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const DT_RELA: i64 = 7;
const DT_RELASZ: i64 = 8;

#[derive(Debug)]
pub enum ElfError {
    InvalidMagic,
    TooSmall,
    NoLoadSegments,
}

pub struct Image {
    pub image: Vec<u8>,
    pub entry: usize,
}

pub fn load_elf(bytes: &[u8], elf_arch: &dyn ElfArch) -> Result<Image, ElfError> {
    if bytes.len() < mem::size_of::<Elf64Header>() {
        return Err(ElfError::TooSmall);
    }

    let header = unsafe { &*(bytes.as_ptr() as *const Elf64Header) };

    if header.e_ident[..4] != ELF_MAGIC {
        return Err(ElfError::InvalidMagic);
    }

    let phdrs = unsafe {
        core::slice::from_raw_parts(
            bytes.as_ptr().add(header.e_phoff as usize) as *const Elf64Phdr,
            header.e_phnum as usize,
        )
    };

    let mut max_addr: u64 = 0;
    for phdr in phdrs {
        if phdr.p_type == PT_LOAD {
            let end = phdr.p_vaddr + phdr.p_memsz;
            if end > max_addr {
                max_addr = end;
            }
        }
    }

    if max_addr == 0 {
        return Err(ElfError::NoLoadSegments);
    }

    let mut image = vec![0u8; max_addr as usize];

    for phdr in phdrs {
        if phdr.p_type == PT_LOAD {
            let dst_start = phdr.p_vaddr as usize;
            let src_start = phdr.p_offset as usize;
            let copy_len = phdr.p_filesz as usize;
            image[dst_start..dst_start + copy_len]
                .copy_from_slice(&bytes[src_start..src_start + copy_len]);
        }
    }

    let base = image.as_ptr() as usize;

    for phdr in phdrs {
        if phdr.p_type == PT_DYNAMIC {
            apply_relocations(&image, base, phdr, elf_arch);
            break;
        }
    }

    let entry = base + header.e_entry as usize;

    Ok(Image { image, entry })
}

fn apply_relocations(image: &[u8], base: usize, dynamic_phdr: &Elf64Phdr, elf_arch: &dyn ElfArch) {
    let dyn_start = dynamic_phdr.p_vaddr as usize;
    let dyn_count = dynamic_phdr.p_memsz as usize / mem::size_of::<Elf64Dyn>();

    let dyns = unsafe {
        core::slice::from_raw_parts(
            image.as_ptr().add(dyn_start) as *const Elf64Dyn,
            dyn_count,
        )
    };

    let mut rela_offset: Option<u64> = None;
    let mut rela_size: Option<u64> = None;

    for dyn_entry in dyns {
        if dyn_entry.d_tag == 0 {
            break;
        }
        match dyn_entry.d_tag {
            DT_RELA => rela_offset = Some(dyn_entry.d_val),
            DT_RELASZ => rela_size = Some(dyn_entry.d_val),
            _ => {}
        }
    }

    if let (Some(offset), Some(size)) = (rela_offset, rela_size) {
        let rela_count = size as usize / mem::size_of::<Elf64Rela>();
        let relas = unsafe {
            core::slice::from_raw_parts(
                image.as_ptr().add(offset as usize) as *const Elf64Rela,
                rela_count,
            )
        };

        for rela in relas {
            elf_arch.apply_relocation(base, rela.r_offset as usize, rela.r_info, rela.r_addend);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::structs::*;
    use std::sync::Mutex;

    struct MockElfArch {
        calls: Mutex<Vec<(usize, usize, u64, i64)>>,
    }

    impl MockElfArch {
        fn new() -> Self {
            Self { calls: Mutex::new(Vec::new()) }
        }

        fn calls(&self) -> Vec<(usize, usize, u64, i64)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl ElfArch for MockElfArch {
        fn apply_relocation(&self, base: usize, offset: usize, info: u64, addend: i64) {
            self.calls.lock().unwrap().push((base, offset, info, addend));
        }
    }

    unsafe fn write_at<T: Copy>(buf: &mut [u8], offset: usize, value: T) {
        let size = mem::size_of::<T>();
        let bytes = unsafe { core::slice::from_raw_parts(&value as *const T as *const u8, size) };
        buf[offset..offset + size].copy_from_slice(bytes);
    }

    fn build_minimal_elf(entry_vaddr: u64) -> Vec<u8> {
        let header_size = mem::size_of::<Elf64Header>();
        let phdr_size = mem::size_of::<Elf64Phdr>();
        let total = header_size + phdr_size + 64;
        let mut buf = vec![0u8; total];

        let header = Elf64Header {
            e_ident: { let mut id = [0u8; 16]; id[..4].copy_from_slice(&ELF_MAGIC); id },
            e_type: 2,
            e_machine: 0x3E,
            e_version: 1,
            e_entry: entry_vaddr,
            e_phoff: header_size as u64,
            e_shoff: 0,
            e_flags: 0,
            e_ehsize: header_size as u16,
            e_phentsize: phdr_size as u16,
            e_phnum: 1,
            e_shentsize: 0,
            e_shnum: 0,
            e_shstrndx: 0,
        };
        let phdr = Elf64Phdr {
            p_type: PT_LOAD,
            p_flags: 5,
            p_offset: 0,
            p_vaddr: 0,
            p_paddr: 0,
            p_filesz: total as u64,
            p_memsz: total as u64,
            p_align: 1,
        };

        unsafe {
            write_at(&mut buf, 0, header);
            write_at(&mut buf, header_size, phdr);
        }

        buf
    }

    fn build_elf_with_rela(r_offset: u64, r_info: u64, r_addend: i64) -> Vec<u8> {
        let header_size = mem::size_of::<Elf64Header>();
        let phdr_size = mem::size_of::<Elf64Phdr>();
        let dyn_size = mem::size_of::<Elf64Dyn>();
        let rela_size = mem::size_of::<Elf64Rela>();

        let load_phdr_at = header_size;
        let dyn_phdr_at = load_phdr_at + phdr_size;
        let dyn_data_at = dyn_phdr_at + phdr_size;
        let rela_data_at = dyn_data_at + 3 * dyn_size;
        let total = rela_data_at + rela_size;

        let mut buf = vec![0u8; total];

        let header = Elf64Header {
            e_ident: { let mut id = [0u8; 16]; id[..4].copy_from_slice(&ELF_MAGIC); id },
            e_type: 2,
            e_machine: 0x3E,
            e_version: 1,
            e_entry: 0,
            e_phoff: load_phdr_at as u64,
            e_shoff: 0,
            e_flags: 0,
            e_ehsize: header_size as u16,
            e_phentsize: phdr_size as u16,
            e_phnum: 2,
            e_shentsize: 0,
            e_shnum: 0,
            e_shstrndx: 0,
        };
        let load_phdr = Elf64Phdr {
            p_type: PT_LOAD,
            p_flags: 5,
            p_offset: 0,
            p_vaddr: 0,
            p_paddr: 0,
            p_filesz: total as u64,
            p_memsz: total as u64,
            p_align: 1,
        };
        let dyn_phdr = Elf64Phdr {
            p_type: PT_DYNAMIC,
            p_flags: 6,
            p_offset: dyn_data_at as u64,
            p_vaddr: dyn_data_at as u64,
            p_paddr: dyn_data_at as u64,
            p_filesz: (3 * dyn_size + rela_size) as u64,
            p_memsz: (3 * dyn_size + rela_size) as u64,
            p_align: 1,
        };

        unsafe {
            write_at(&mut buf, 0, header);
            write_at(&mut buf, load_phdr_at, load_phdr);
            write_at(&mut buf, dyn_phdr_at, dyn_phdr);
            write_at(&mut buf, dyn_data_at, Elf64Dyn { d_tag: DT_RELA, d_val: rela_data_at as u64 });
            write_at(&mut buf, dyn_data_at + dyn_size, Elf64Dyn { d_tag: DT_RELASZ, d_val: rela_size as u64 });
            write_at(&mut buf, dyn_data_at + 2 * dyn_size, Elf64Dyn { d_tag: 0, d_val: 0 });
            write_at(&mut buf, rela_data_at, Elf64Rela { r_offset, r_info, r_addend });
        }

        buf
    }

    #[test]
    fn rejects_too_small() {
        let mock = MockElfArch::new();
        assert!(matches!(load_elf(&[], &mock), Err(ElfError::TooSmall)));
        assert!(matches!(load_elf(&[0u8; 10], &mock), Err(ElfError::TooSmall)));
    }

    #[test]
    fn rejects_invalid_magic() {
        let mock = MockElfArch::new();
        let buf = vec![0u8; mem::size_of::<Elf64Header>()];
        assert!(matches!(load_elf(&buf, &mock), Err(ElfError::InvalidMagic)));
    }

    #[test]
    fn rejects_no_load_segments() {
        let mock = MockElfArch::new();
        let header_size = mem::size_of::<Elf64Header>();
        let mut buf = vec![0u8; header_size];
        let header = Elf64Header {
            e_ident: { let mut id = [0u8; 16]; id[..4].copy_from_slice(&ELF_MAGIC); id },
            e_type: 2,
            e_machine: 0x3E,
            e_version: 1,
            e_entry: 0,
            e_phoff: header_size as u64,
            e_shoff: 0,
            e_flags: 0,
            e_ehsize: header_size as u16,
            e_phentsize: mem::size_of::<Elf64Phdr>() as u16,
            e_phnum: 0,
            e_shentsize: 0,
            e_shnum: 0,
            e_shstrndx: 0,
        };
        unsafe { write_at(&mut buf, 0, header); }
        assert!(matches!(load_elf(&buf, &mock), Err(ElfError::NoLoadSegments)));
    }

    #[test]
    fn computes_entry_point_from_base_and_e_entry() {
        let mock = MockElfArch::new();
        let elf = build_minimal_elf(42);
        let image = load_elf(&elf, &mock).unwrap();
        assert_eq!(image.entry, image.image.as_ptr() as usize + 42);
    }

    #[test]
    fn delegates_all_rela_entries_to_elf_arch() {
        let mock = MockElfArch::new();
        let elf = build_elf_with_rela(0x10, 0xAB_0000_0008, 100);
        let image = load_elf(&elf, &mock).unwrap();

        let calls = mock.calls();
        assert_eq!(calls.len(), 1);

        let (base, offset, info, addend) = calls[0];
        assert_eq!(base, image.image.as_ptr() as usize);
        assert_eq!(offset, 0x10);
        assert_eq!(info, 0xAB_0000_0008);
        assert_eq!(addend, 100);
    }

    #[test]
    fn no_relocations_called_without_dynamic_segment() {
        let mock = MockElfArch::new();
        let elf = build_minimal_elf(0);
        load_elf(&elf, &mock).unwrap();
        assert!(mock.calls().is_empty());
    }
}
