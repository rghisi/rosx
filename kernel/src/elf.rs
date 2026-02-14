use alloc::vec::Vec;
use core::mem;

use crate::kernel::task_wrapper;
use crate::task::{SharedTask, Task, next_id};

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const DT_RELA: i64 = 7;
const DT_RELASZ: i64 = 8;
const R_X86_64_RELATIVE: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64Header {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64Phdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64Dyn {
    d_tag: i64,
    d_val: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64Rela {
    r_offset: u64,
    r_info: u64,
    r_addend: i64,
}

impl Elf64Rela {
    fn r_type(&self) -> u32 {
        (self.r_info & 0xffffffff) as u32
    }
}

#[derive(Debug)]
pub enum ElfError {
    InvalidMagic,
    TooSmall,
    NoLoadSegments,
}

pub fn load_elf(bytes: &[u8]) -> Result<SharedTask, ElfError> {
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

    let image_size = max_addr as usize;
    let mut image = Vec::<u8>::with_capacity(image_size);
    image.resize(image_size, 0);

    for phdr in phdrs {
        if phdr.p_type == PT_LOAD {
            let dst_start = phdr.p_vaddr as usize;
            let src_start = phdr.p_offset as usize;
            let copy_len = phdr.p_filesz as usize;
            image[dst_start..dst_start + copy_len]
                .copy_from_slice(&bytes[src_start..src_start + copy_len]);
        }
    }

    let base = image.as_ptr() as u64;

    for phdr in phdrs {
        if phdr.p_type == PT_DYNAMIC {
            apply_relocations(&image, base, phdr);
            break;
        }
    }

    let entry = base + header.e_entry;
    let image_leaked = image.leak();
    let _ = image_leaked;

    Ok(Task::new(next_id(), "ELF", task_wrapper as usize, entry as usize))
}

fn apply_relocations(image: &[u8], base: u64, dynamic_phdr: &Elf64Phdr) {
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
            if rela.r_type() == R_X86_64_RELATIVE {
                let patch_addr = (base + rela.r_offset) as *mut u64;
                let value = (base as i64 + rela.r_addend) as u64;
                unsafe {
                    core::ptr::write(patch_addr, value);
                }
            }
        }
    }
}
