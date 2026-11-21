pub(crate) mod structs;

use alloc::vec;
use alloc::vec::Vec;
use core::mem;
use crate::platform::elf::ElfArch;
use structs::{Elf32Dyn, Elf32Header, Elf32Phdr, Elf32Rel, Elf64Dyn, Elf64Header, Elf64Phdr, Elf64Rela};

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const EI_CLASS: usize = 4;
const ELFCLASS32: u8 = 1;
const ELFCLASS64: u8 = 2;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const DT_RELA: i64 = 7;
const DT_RELASZ: i64 = 8;
const DT_REL: i32 = 17;
const DT_RELSZ: i32 = 18;

const MAX_IMAGE_SPAN: usize = 4 * 1024 * 1024;
const MIN_IMAGE_ALIGN: usize = 16;
const MAX_IMAGE_ALIGN: usize = 4096;

#[derive(Debug)]
pub enum ElfError {
    InvalidMagic,
    TooSmall,
    NoLoadSegments,
    UnsupportedClass,
    MalformedSegment,
    SpanTooLarge,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ElfClass {
    Elf32,
    Elf64,
}

#[derive(Debug, Copy, Clone)]
pub struct ElfLayout {
    pub span: usize,
    pub align: usize,
    pub entry_offset: usize,
    pub class: ElfClass,
}

pub struct Image {
    pub image: Vec<u8>,
    pub entry: usize,
}

fn read_at<T: Copy>(bytes: &[u8], offset: usize) -> Option<T> {
    let end = offset.checked_add(mem::size_of::<T>())?;
    if end > bytes.len() {
        return None;
    }
    Some(unsafe { core::ptr::read_unaligned(bytes.as_ptr().add(offset) as *const T) })
}

pub fn inspect(bytes: &[u8]) -> Result<ElfLayout, ElfError> {
    if bytes.len() < mem::size_of::<Elf32Header>() {
        return Err(ElfError::TooSmall);
    }

    if bytes[..4] != ELF_MAGIC {
        return Err(ElfError::InvalidMagic);
    }

    match bytes[EI_CLASS] {
        ELFCLASS32 => inspect32(bytes),
        ELFCLASS64 => {
            if bytes.len() < mem::size_of::<Elf64Header>() {
                return Err(ElfError::TooSmall);
            }
            inspect64(bytes)
        }
        _ => Err(ElfError::UnsupportedClass),
    }
}

pub fn load_into(
    bytes: &[u8],
    dst: &mut [u8],
    layout: &ElfLayout,
    elf_arch: &dyn ElfArch,
) -> Result<usize, ElfError> {
    match layout.class {
        ElfClass::Elf32 => load_into32(bytes, dst, layout, elf_arch),
        ElfClass::Elf64 => load_into64(bytes, dst, layout, elf_arch),
    }
}

fn inspect32(bytes: &[u8]) -> Result<ElfLayout, ElfError> {
    let header: Elf32Header = read_at(bytes, 0).ok_or(ElfError::TooSmall)?;
    let phdrs = phdr_table32(bytes, header.e_phoff, header.e_phnum)?;

    let mut span: u32 = 0;
    let mut align: u32 = 1;
    for phdr in &phdrs {
        if phdr.p_type != PT_LOAD {
            continue;
        }
        if phdr.p_filesz > phdr.p_memsz {
            return Err(ElfError::MalformedSegment);
        }
        let span_end = phdr.p_vaddr.checked_add(phdr.p_memsz).ok_or(ElfError::MalformedSegment)?;
        let file_end = phdr.p_offset.checked_add(phdr.p_filesz).ok_or(ElfError::MalformedSegment)?;
        if (file_end as u64) > bytes.len() as u64 {
            return Err(ElfError::MalformedSegment);
        }
        span = span.max(span_end);
        if phdr.p_align >= 2 && phdr.p_align.is_power_of_two() {
            align = align.max(phdr.p_align);
        }
    }

    if span == 0 {
        return Err(ElfError::NoLoadSegments);
    }
    if span as usize > MAX_IMAGE_SPAN {
        return Err(ElfError::SpanTooLarge);
    }
    if header.e_entry >= span {
        return Err(ElfError::MalformedSegment);
    }

    Ok(ElfLayout {
        span: span as usize,
        align: clamp_align(align),
        entry_offset: header.e_entry as usize,
        class: ElfClass::Elf32,
    })
}

fn inspect64(bytes: &[u8]) -> Result<ElfLayout, ElfError> {
    let header: Elf64Header = read_at(bytes, 0).ok_or(ElfError::TooSmall)?;
    let phdrs = phdr_table64(bytes, header.e_phoff, header.e_phnum)?;

    let mut span: u64 = 0;
    let mut align: u64 = 1;
    for phdr in &phdrs {
        if phdr.p_type != PT_LOAD {
            continue;
        }
        if phdr.p_filesz > phdr.p_memsz {
            return Err(ElfError::MalformedSegment);
        }
        let span_end = phdr.p_vaddr.checked_add(phdr.p_memsz).ok_or(ElfError::MalformedSegment)?;
        let file_end = phdr.p_offset.checked_add(phdr.p_filesz).ok_or(ElfError::MalformedSegment)?;
        if file_end > bytes.len() as u64 {
            return Err(ElfError::MalformedSegment);
        }
        span = span.max(span_end);
        if phdr.p_align >= 2 && phdr.p_align.is_power_of_two() {
            align = align.max(phdr.p_align);
        }
    }

    if span == 0 {
        return Err(ElfError::NoLoadSegments);
    }
    if span > MAX_IMAGE_SPAN as u64 {
        return Err(ElfError::SpanTooLarge);
    }
    if header.e_entry >= span {
        return Err(ElfError::MalformedSegment);
    }

    Ok(ElfLayout {
        span: span as usize,
        align: clamp_align(align),
        entry_offset: header.e_entry as usize,
        class: ElfClass::Elf64,
    })
}

fn clamp_align(align: impl TryInto<usize> + Copy) -> usize {
    let requested: usize = align.try_into().unwrap_or(MAX_IMAGE_ALIGN);
    requested.clamp(MIN_IMAGE_ALIGN, MAX_IMAGE_ALIGN)
}

fn phdr_table32(bytes: &[u8], phoff: u32, phnum: u16) -> Result<Vec<Elf32Phdr>, ElfError> {
    let entry = mem::size_of::<Elf32Phdr>();
    let table_size = (phnum as usize)
        .checked_mul(entry)
        .ok_or(ElfError::MalformedSegment)?;
    let start = phoff as usize;
    let end = start.checked_add(table_size).ok_or(ElfError::MalformedSegment)?;
    if end > bytes.len() {
        return Err(ElfError::MalformedSegment);
    }
    (0..phnum as usize)
        .map(|i| read_at(bytes, start + i * entry).ok_or(ElfError::MalformedSegment))
        .collect()
}

fn phdr_table64(bytes: &[u8], phoff: u64, phnum: u16) -> Result<Vec<Elf64Phdr>, ElfError> {
    let entry = mem::size_of::<Elf64Phdr>();
    let table_size = (phnum as usize)
        .checked_mul(entry)
        .ok_or(ElfError::MalformedSegment)?;
    let start = usize::try_from(phoff).map_err(|_| ElfError::MalformedSegment)?;
    let end = start.checked_add(table_size).ok_or(ElfError::MalformedSegment)?;
    if end > bytes.len() {
        return Err(ElfError::MalformedSegment);
    }
    (0..phnum as usize)
        .map(|i| read_at(bytes, start + i * entry).ok_or(ElfError::MalformedSegment))
        .collect()
}

fn load_into32(
    bytes: &[u8],
    dst: &mut [u8],
    layout: &ElfLayout,
    elf_arch: &dyn ElfArch,
) -> Result<usize, ElfError> {
    let header: Elf32Header = read_at(bytes, 0).ok_or(ElfError::TooSmall)?;
    let phdrs = phdr_table32(bytes, header.e_phoff, header.e_phnum)?;
    debug_assert_eq!(dst.len(), layout.span);

    for phdr in &phdrs {
        if phdr.p_type != PT_LOAD {
            continue;
        }
        let dst_start = phdr.p_vaddr as usize;
        let copy_len = phdr.p_filesz as usize;
        let src_start = phdr.p_offset as usize;
        if src_start.checked_add(copy_len).ok_or(ElfError::MalformedSegment)? > bytes.len()
            || dst_start.checked_add(copy_len).ok_or(ElfError::MalformedSegment)? > dst.len()
        {
            return Err(ElfError::MalformedSegment);
        }
        dst[dst_start..dst_start + copy_len].copy_from_slice(&bytes[src_start..src_start + copy_len]);
    }

    let base = dst.as_ptr() as usize;
    for phdr in &phdrs {
        if phdr.p_type == PT_DYNAMIC {
            apply_relocations32(dst, base, phdr, elf_arch)?;
            break;
        }
    }

    Ok(base + layout.entry_offset)
}

fn load_into64(
    bytes: &[u8],
    dst: &mut [u8],
    layout: &ElfLayout,
    elf_arch: &dyn ElfArch,
) -> Result<usize, ElfError> {
    let header: Elf64Header = read_at(bytes, 0).ok_or(ElfError::TooSmall)?;
    let phdrs = phdr_table64(bytes, header.e_phoff, header.e_phnum)?;
    debug_assert_eq!(dst.len(), layout.span);

    for phdr in &phdrs {
        if phdr.p_type != PT_LOAD {
            continue;
        }
        let dst_start = phdr.p_vaddr as usize;
        let copy_len = phdr.p_filesz as usize;
        let src_start = phdr.p_offset as usize;
        if src_start.checked_add(copy_len).ok_or(ElfError::MalformedSegment)? > bytes.len()
            || dst_start.checked_add(copy_len).ok_or(ElfError::MalformedSegment)? > dst.len()
        {
            return Err(ElfError::MalformedSegment);
        }
        dst[dst_start..dst_start + copy_len].copy_from_slice(&bytes[src_start..src_start + copy_len]);
    }

    let base = dst.as_ptr() as usize;
    for phdr in &phdrs {
        if phdr.p_type == PT_DYNAMIC {
            apply_relocations64(dst, base, phdr, elf_arch)?;
            break;
        }
    }

    Ok(base + layout.entry_offset)
}

fn apply_relocations64(
    image: &[u8],
    base: usize,
    dynamic_phdr: &Elf64Phdr,
    elf_arch: &dyn ElfArch,
) -> Result<(), ElfError> {
    let dyn_start = usize::try_from(dynamic_phdr.p_vaddr).map_err(|_| ElfError::MalformedSegment)?;
    let dyn_size = usize::try_from(dynamic_phdr.p_memsz).map_err(|_| ElfError::MalformedSegment)?;
    if dyn_start.checked_add(dyn_size).ok_or(ElfError::MalformedSegment)? > image.len() {
        return Err(ElfError::MalformedSegment);
    }

    let mut rela_offset: Option<usize> = None;
    let mut rela_size: Option<usize> = None;
    for i in 0..dyn_size / mem::size_of::<Elf64Dyn>() {
        let dyn_entry: Elf64Dyn =
            read_at(image, dyn_start + i * mem::size_of::<Elf64Dyn>()).ok_or(ElfError::MalformedSegment)?;
        if dyn_entry.d_tag == 0 {
            break;
        }
        match dyn_entry.d_tag {
            DT_RELA => {
                rela_offset =
                    Some(usize::try_from(dyn_entry.d_val).map_err(|_| ElfError::MalformedSegment)?);
            }
            DT_RELASZ => {
                rela_size =
                    Some(usize::try_from(dyn_entry.d_val).map_err(|_| ElfError::MalformedSegment)?);
            }
            _ => {}
        }
    }

    if let (Some(offset), Some(size)) = (rela_offset, rela_size) {
        let entry = mem::size_of::<Elf64Rela>();
        if offset.checked_add(size).ok_or(ElfError::MalformedSegment)? > image.len() {
            return Err(ElfError::MalformedSegment);
        }
        for i in 0..size / entry {
            let rela: Elf64Rela =
                read_at(image, offset + i * entry).ok_or(ElfError::MalformedSegment)?;
            let r_offset = usize::try_from(rela.r_offset).map_err(|_| ElfError::MalformedSegment)?;
            if r_offset.checked_add(8).ok_or(ElfError::MalformedSegment)? > image.len() {
                return Err(ElfError::MalformedSegment);
            }
            elf_arch.apply_relocation(base, r_offset, rela.r_info, rela.r_addend);
        }
    }
    Ok(())
}

fn apply_relocations32(
    image: &[u8],
    base: usize,
    dynamic_phdr: &Elf32Phdr,
    elf_arch: &dyn ElfArch,
) -> Result<(), ElfError> {
    let dyn_start = dynamic_phdr.p_vaddr as usize;
    let dyn_size = dynamic_phdr.p_memsz as usize;
    if dyn_start.checked_add(dyn_size).ok_or(ElfError::MalformedSegment)? > image.len() {
        return Err(ElfError::MalformedSegment);
    }

    let mut rel_offset: Option<usize> = None;
    let mut rel_size: Option<usize> = None;
    for i in 0..dyn_size / mem::size_of::<Elf32Dyn>() {
        let dyn_entry: Elf32Dyn =
            read_at(image, dyn_start + i * mem::size_of::<Elf32Dyn>()).ok_or(ElfError::MalformedSegment)?;
        if dyn_entry.d_tag == 0 {
            break;
        }
        match dyn_entry.d_tag {
            DT_REL => rel_offset = Some(dyn_entry.d_val as usize),
            DT_RELSZ => rel_size = Some(dyn_entry.d_val as usize),
            _ => {}
        }
    }

    if let (Some(offset), Some(size)) = (rel_offset, rel_size) {
        let entry = mem::size_of::<Elf32Rel>();
        if offset.checked_add(size).ok_or(ElfError::MalformedSegment)? > image.len() {
            return Err(ElfError::MalformedSegment);
        }
        for i in 0..size / entry {
            let rel: Elf32Rel =
                read_at(image, offset + i * entry).ok_or(ElfError::MalformedSegment)?;
            let patch_offset = rel.r_offset as usize;
            if patch_offset.checked_add(4).ok_or(ElfError::MalformedSegment)? > image.len() {
                return Err(ElfError::MalformedSegment);
            }
            let implicit_addend = read_at::<i32>(image, patch_offset)
                .ok_or(ElfError::MalformedSegment)? as i64;
            elf_arch.apply_relocation(base, patch_offset, rel.r_info as u64, implicit_addend);
        }
    }
    Ok(())
}

pub fn load_elf(bytes: &[u8], elf_arch: &dyn ElfArch) -> Result<Image, ElfError> {
    let layout = inspect(bytes)?;
    let mut image = vec![0u8; layout.span];
    let entry = load_into(bytes, &mut image, &layout, elf_arch)?;
    Ok(Image { image, entry })
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
            e_ident: { let mut id = [0u8; 16]; id[..4].copy_from_slice(&ELF_MAGIC); id[4] = 2; id },
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

    fn phdr_slot64() -> usize {
        mem::size_of::<Elf64Header>()
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
            e_ident: { let mut id = [0u8; 16]; id[..4].copy_from_slice(&ELF_MAGIC); id[4] = 2; id },
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
            e_ident: { let mut id = [0u8; 16]; id[..4].copy_from_slice(&ELF_MAGIC); id[4] = 2; id },
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

    #[test]
    fn rejects_unsupported_elf_class() {
        let mock = MockElfArch::new();
        let mut buf = vec![0u8; mem::size_of::<Elf64Header>()];
        buf[..4].copy_from_slice(&ELF_MAGIC);
        buf[4] = 0; // ELFCLASSNONE
        assert!(matches!(load_elf(&buf, &mock), Err(ElfError::UnsupportedClass)));
    }

    fn build_elf32_minimal(entry_vaddr: u32) -> Vec<u8> {
        let header_size = mem::size_of::<Elf32Header>();
        let phdr_size = mem::size_of::<Elf32Phdr>();
        let total = header_size + phdr_size + 32;
        let mut buf = vec![0u8; total];

        let header = Elf32Header {
            e_ident: { let mut id = [0u8; 16]; id[..4].copy_from_slice(&ELF_MAGIC); id[4] = 1; id },
            e_type: 2,
            e_machine: 3,
            e_version: 1,
            e_entry: entry_vaddr,
            e_phoff: header_size as u32,
            e_shoff: 0,
            e_flags: 0,
            e_ehsize: header_size as u16,
            e_phentsize: phdr_size as u16,
            e_phnum: 1,
            e_shentsize: 0,
            e_shnum: 0,
            e_shstrndx: 0,
        };
        let phdr = Elf32Phdr {
            p_type: PT_LOAD,
            p_offset: 0,
            p_vaddr: 0,
            p_paddr: 0,
            p_filesz: total as u32,
            p_memsz: total as u32,
            p_flags: 5,
            p_align: 1,
        };

        unsafe {
            write_at(&mut buf, 0, header);
            write_at(&mut buf, header_size, phdr);
        }
        buf
    }

    fn build_elf32_with_rel(r_offset: u32, r_info: u32, implicit_addend: u32) -> Vec<u8> {
        let header_size = mem::size_of::<Elf32Header>();
        let phdr_size = mem::size_of::<Elf32Phdr>();
        let dyn_size = mem::size_of::<Elf32Dyn>();
        let rel_size = mem::size_of::<Elf32Rel>();

        let load_phdr_at = header_size;
        let dyn_phdr_at = load_phdr_at + phdr_size;
        let dyn_data_at = dyn_phdr_at + phdr_size;
        let rel_data_at = dyn_data_at + 3 * dyn_size;
        let total = rel_data_at + rel_size;

        let mut buf = vec![0u8; total];

        let header = Elf32Header {
            e_ident: { let mut id = [0u8; 16]; id[..4].copy_from_slice(&ELF_MAGIC); id[4] = 1; id },
            e_type: 2,
            e_machine: 3,
            e_version: 1,
            e_entry: 0,
            e_phoff: load_phdr_at as u32,
            e_shoff: 0,
            e_flags: 0,
            e_ehsize: header_size as u16,
            e_phentsize: phdr_size as u16,
            e_phnum: 2,
            e_shentsize: 0,
            e_shnum: 0,
            e_shstrndx: 0,
        };
        let load_phdr = Elf32Phdr {
            p_type: PT_LOAD,
            p_offset: 0,
            p_vaddr: 0,
            p_paddr: 0,
            p_filesz: total as u32,
            p_memsz: total as u32,
            p_flags: 5,
            p_align: 1,
        };
        let dyn_phdr = Elf32Phdr {
            p_type: PT_DYNAMIC,
            p_offset: dyn_data_at as u32,
            p_vaddr: dyn_data_at as u32,
            p_paddr: dyn_data_at as u32,
            p_filesz: (3 * dyn_size + rel_size) as u32,
            p_memsz: (3 * dyn_size + rel_size) as u32,
            p_flags: 6,
            p_align: 1,
        };

        unsafe {
            write_at(&mut buf, 0, header);
            write_at(&mut buf, load_phdr_at, load_phdr);
            write_at(&mut buf, dyn_phdr_at, dyn_phdr);
            write_at(&mut buf, dyn_data_at,              Elf32Dyn { d_tag: 17, d_val: rel_data_at as u32 }); // DT_REL
            write_at(&mut buf, dyn_data_at + dyn_size,   Elf32Dyn { d_tag: 18, d_val: rel_size as u32 });    // DT_RELSZ
            write_at(&mut buf, dyn_data_at + 2 * dyn_size, Elf32Dyn { d_tag: 0, d_val: 0 });
            write_at(&mut buf, rel_data_at, Elf32Rel { r_offset, r_info });
            write_at(&mut buf, r_offset as usize, implicit_addend);
        }
        buf
    }

    #[test]
    fn load_elf32_loads_single_segment() {
        let mock = MockElfArch::new();
        let elf = build_elf32_minimal(0);
        let image = load_elf(&elf, &mock).unwrap();
        assert_eq!(&image.image[..4], &[0x7f, b'E', b'L', b'F']);
    }

    #[test]
    fn load_elf32_computes_entry_from_base_plus_e_entry() {
        let mock = MockElfArch::new();
        let elf = build_elf32_minimal(42);
        let image = load_elf(&elf, &mock).unwrap();
        assert_eq!(image.entry, image.image.as_ptr() as usize + 42);
    }

    #[test]
    fn load_elf32_delegates_rel_entries_to_elf_arch() {
        let mock = MockElfArch::new();
        let elf = build_elf32_with_rel(0x10, 0x08, 0x5000);
        let image = load_elf(&elf, &mock).unwrap();

        let calls = mock.calls();
        assert_eq!(calls.len(), 1);

        let (base, offset, info, addend) = calls[0];
        assert_eq!(base, image.image.as_ptr() as usize);
        assert_eq!(offset, 0x10);
        assert_eq!(info, 0x08);
        assert_eq!(addend, 0x5000);
    }

    #[test]
    fn inspect_reports_the_minimal_elf64_layout() {
        let elf = build_minimal_elf(42);
        let layout = inspect(&elf).unwrap();
        assert_eq!(layout.span, elf.len());
        assert_eq!(layout.entry_offset, 42);
        assert_eq!(layout.align, MIN_IMAGE_ALIGN);
        assert_eq!(layout.class, ElfClass::Elf64);
    }

    #[test]
    fn inspect_reports_the_minimal_elf32_layout() {
        let elf = build_elf32_minimal(42);
        let layout = inspect(&elf).unwrap();
        assert_eq!(layout.span, elf.len());
        assert_eq!(layout.entry_offset, 42);
        assert_eq!(layout.class, ElfClass::Elf32);
    }

    #[test]
    fn inspect_rejects_the_header_rejections_as_inspect() {
        assert!(matches!(inspect(&[]), Err(ElfError::TooSmall)));
        assert!(matches!(inspect(&[0u8; 10]), Err(ElfError::TooSmall)));
        let mut buf = vec![0u8; mem::size_of::<Elf64Header>()];
        assert!(matches!(inspect(&buf), Err(ElfError::InvalidMagic)));
        buf[..4].copy_from_slice(&ELF_MAGIC);
        buf[4] = 0;
        assert!(matches!(inspect(&buf), Err(ElfError::UnsupportedClass)));
    }

    #[test]
    fn inspect_rejects_a_load_segment_extending_past_the_file() {
        let mut elf = build_minimal_elf(0);
        let mut phdr: Elf64Phdr = read_at(&elf, phdr_slot64()).unwrap();
        phdr.p_filesz = (elf.len() + 1) as u64;
        unsafe { write_at(&mut elf, phdr_slot64(), phdr) };
        assert!(matches!(inspect(&elf), Err(ElfError::MalformedSegment)));
    }

    #[test]
    fn inspect_rejects_filesz_above_memsz() {
        let mut elf = build_minimal_elf(0);
        let mut phdr: Elf64Phdr = read_at(&elf, phdr_slot64()).unwrap();
        phdr.p_memsz = (elf.len() / 2) as u64;
        unsafe { write_at(&mut elf, phdr_slot64(), phdr) };
        assert!(matches!(inspect(&elf), Err(ElfError::MalformedSegment)));
    }

    #[test]
    fn inspect_rejects_overflowing_segment_bounds() {
        let mut elf = build_minimal_elf(0);
        let mut phdr: Elf64Phdr = read_at(&elf, phdr_slot64()).unwrap();
        phdr.p_vaddr = u64::MAX - 2;
        phdr.p_memsz = 4096;
        unsafe { write_at(&mut elf, phdr_slot64(), phdr) };
        assert!(matches!(inspect(&elf), Err(ElfError::MalformedSegment)));
    }

    #[test]
    fn inspect_rejects_a_phdr_table_reaching_past_the_file() {
        let mut elf = build_minimal_elf(0);
        let e_phnum = 56;
        elf[e_phnum..e_phnum + 2].copy_from_slice(&255u16.to_le_bytes());
        assert!(matches!(inspect(&elf), Err(ElfError::MalformedSegment)));
    }

    #[test]
    fn inspect_rejects_a_span_above_the_cap() {
        let mut elf = build_minimal_elf(0);
        let mut phdr: Elf64Phdr = read_at(&elf, phdr_slot64()).unwrap();
        phdr.p_filesz = 0;
        phdr.p_memsz = MAX_IMAGE_SPAN as u64 + 1;
        unsafe { write_at(&mut elf, phdr_slot64(), phdr) };
        assert!(matches!(inspect(&elf), Err(ElfError::SpanTooLarge)));
    }

    #[test]
    fn inspect_rejects_an_entry_point_at_or_past_the_span() {
        let mut elf = build_minimal_elf(0);
        let span: u64 = elf.len() as u64;
        elf[24..32].copy_from_slice(&span.to_le_bytes());
        assert!(matches!(inspect(&elf), Err(ElfError::MalformedSegment)));
    }

    #[test]
    fn inspect_adopts_the_largest_pt_load_alignment_capped_at_the_page() {
        let mut elf = build_minimal_elf(0);
        let mut phdr: Elf64Phdr = read_at(&elf, phdr_slot64()).unwrap();
        phdr.p_align = 16;
        unsafe { write_at(&mut elf, phdr_slot64(), phdr) };
        assert_eq!(inspect(&elf).unwrap().align, 16);

        let mut elf = build_minimal_elf(0);
        let mut phdr: Elf64Phdr = read_at(&elf, phdr_slot64()).unwrap();
        phdr.p_align = 0x20_0000;
        unsafe { write_at(&mut elf, phdr_slot64(), phdr) };
        assert_eq!(inspect(&elf).unwrap().align, MAX_IMAGE_ALIGN);

        let mut elf = build_minimal_elf(0);
        let mut phdr: Elf64Phdr = read_at(&elf, phdr_slot64()).unwrap();
        phdr.p_align = 6;
        unsafe { write_at(&mut elf, phdr_slot64(), phdr) };
        assert_eq!(inspect(&elf).unwrap().align, MIN_IMAGE_ALIGN);
    }

    #[test]
    fn load_into64_reproduces_the_file_bytes_for_one_covering_segment() {
        let mock = MockElfArch::new();
        let elf = build_minimal_elf(42);
        let layout = inspect(&elf).unwrap();
        let mut dst = vec![0u8; layout.span];
        let entry = load_into(&elf, &mut dst, &layout, &mock).unwrap();
        assert_eq!(dst, elf);
        assert_eq!(entry, dst.as_ptr() as usize + 42);
    }

    #[test]
    fn load_into32_reproduces_the_file_bytes_for_one_covering_segment() {
        let mock = MockElfArch::new();
        let elf = build_elf32_minimal(42);
        let layout = inspect(&elf).unwrap();
        let mut dst = vec![0u8; layout.span];
        let entry = load_into(&elf, &mut dst, &layout, &mock).unwrap();
        assert_eq!(dst, elf);
        assert_eq!(entry, dst.as_ptr() as usize + 42);
    }

    #[test]
    fn load_into64_applies_relocations_against_the_destination() {
        let mock = MockElfArch::new();
        let elf = build_elf_with_rela(0x10, 0xAB_0000_0008, 100);
        let layout = inspect(&elf).unwrap();
        let mut dst = vec![0u8; layout.span];
        let entry = load_into(&elf, &mut dst, &layout, &mock).unwrap();
        assert_eq!(dst, elf);
        assert_eq!(entry, dst.as_ptr() as usize);

        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        let (base, offset, info, addend) = calls[0];
        assert_eq!(base, dst.as_ptr() as usize);
        assert_eq!(offset, 0x10);
        assert_eq!(info, 0xAB_0000_0008);
        assert_eq!(addend, 100);
    }

    #[test]
    fn load_into32_applies_relocations_against_the_destination() {
        let mock = MockElfArch::new();
        let elf = build_elf32_with_rel(0x10, 0x08, 0x5000);
        let layout = inspect(&elf).unwrap();
        let mut dst = vec![0u8; layout.span];
        let entry = load_into(&elf, &mut dst, &layout, &mock).unwrap();
        assert_eq!(dst, elf);
        assert_eq!(entry, dst.as_ptr() as usize);

        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        let (base, offset, info, addend) = calls[0];
        assert_eq!(base, dst.as_ptr() as usize);
        assert_eq!(offset, 0x10);
        assert_eq!(info, 0x08);
        assert_eq!(addend, 0x5000);
    }

    fn build_elf_with_rela_pointing(dyn_data_at: usize, rela_val: u64, relasz_val: u64) -> Vec<u8> {
        let mut elf = build_elf_with_rela(0x10, 0xAB_0000_0008, 100);
        let dyn_size = mem::size_of::<Elf64Dyn>();
        unsafe {
            write_at(&mut elf, dyn_data_at, Elf64Dyn { d_tag: DT_RELA, d_val: rela_val });
            write_at(&mut elf, dyn_data_at + dyn_size, Elf64Dyn { d_tag: DT_RELASZ, d_val: relasz_val });
        }
        elf
    }

    fn dyn_data_at_of() -> usize {
        mem::size_of::<Elf64Header>() + 2 * mem::size_of::<Elf64Phdr>()
    }

    #[test]
    fn load_into_rejects_a_rela_table_reaching_past_the_span() {
        let mock = MockElfArch::new();
        let elf = build_elf_with_rela_pointing(dyn_data_at_of(), 0xFFFF_FFFF_FFFF_0000, 24);
        let layout = inspect(&elf).unwrap();
        let mut dst = vec![0u8; layout.span];
        assert!(matches!(load_into(&elf, &mut dst, &layout, &mock), Err(ElfError::MalformedSegment)));
        assert!(mock.calls().is_empty());
    }

    #[test]
    fn load_into_rejects_a_reloc_offset_reaching_past_the_span() {
        let mock = MockElfArch::new();
        let elf = build_elf_with_rela(u64::MAX - 2, 0xAB_0000_0008, 100);
        let layout = inspect(&elf).unwrap();
        let mut dst = vec![0u8; layout.span];
        assert!(matches!(load_into(&elf, &mut dst, &layout, &mock), Err(ElfError::MalformedSegment)));
        assert!(mock.calls().is_empty());
    }
}
