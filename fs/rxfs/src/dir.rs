use crate::inode::{KIND_DIRECTORY, KIND_REGULAR};
use crate::spec::{BLOCK_BYTES, MAX_NAME_BYTES};
use alloc::vec::Vec;

pub const MIN_ENTRY_BYTES: usize = 12;
pub const MAX_ENTRY_BYTES: usize = 264;
pub const ENTRY_HEADER_BYTES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry<'a> {
    pub inode: u32,
    pub kind: u8,
    pub name: &'a [u8],
}

pub fn entry_size(name_len: usize) -> usize {
    (ENTRY_HEADER_BYTES + name_len + 3) & !3
}

pub fn init_empty(block: &mut [u8]) {
    block[..BLOCK_BYTES].fill(0);
    write_header(block, 0, BLOCK_BYTES as u16, 0, 0);
}

pub fn init_dot_entries(block: &mut [u8], self_inode: u32, parent_inode: u32) {
    block[..BLOCK_BYTES].fill(0);
    write_entry(block, 0, self_inode, KIND_DIRECTORY, b".", MIN_ENTRY_BYTES as u16);
    write_entry(
        block,
        MIN_ENTRY_BYTES,
        parent_inode,
        KIND_DIRECTORY,
        b"..",
        BLOCK_BYTES as u16 - MIN_ENTRY_BYTES as u16,
    );
}

pub fn find<'a>(block: &'a [u8], name: &[u8]) -> Option<Entry<'a>> {
    entries_iter(block).find(|entry| entry.inode != 0 && entry.name == name)
}

pub fn entries(block: &[u8]) -> Vec<Entry<'_>> {
    entries_iter(block).filter(|entry| entry.inode != 0).collect()
}

pub fn is_well_formed(block: &[u8]) -> bool {
    let mut pos = 0usize;
    while pos < BLOCK_BYTES {
        let (inode, rec_len, name_len, kind) = read_header(block, pos);
        if rec_len < MIN_ENTRY_BYTES as u16 || rec_len % 4 != 0 {
            return false;
        }
        if pos + rec_len as usize > BLOCK_BYTES {
            return false;
        }
        if inode != 0 {
            let name_ok = name_len >= 1
                && entry_size(name_len as usize) <= rec_len as usize
                && pos + ENTRY_HEADER_BYTES + name_len as usize <= BLOCK_BYTES;
            let kind_ok = kind == KIND_REGULAR || kind == KIND_DIRECTORY;
            if !name_ok || !kind_ok {
                return false;
            }
        }
        pos += rec_len as usize;
    }
    pos == BLOCK_BYTES
}

pub fn add(block: &mut [u8], inode: u32, kind: u8, name: &[u8]) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_BYTES || inode == 0 {
        return false;
    }
    let size = entry_size(name.len());
    let mut pos = 0usize;
    while pos < BLOCK_BYTES {
        let (entry_inode, rec_len, name_len, _) = read_header(block, pos);
        if rec_len < MIN_ENTRY_BYTES as u16 || rec_len % 4 != 0 {
            return false;
        }
        if pos + rec_len as usize > BLOCK_BYTES {
            return false;
        }
        if entry_inode == 0 {
            let rec_len = rec_len as usize;
            if rec_len >= size {
                let leftover = rec_len - size;
                if leftover >= MIN_ENTRY_BYTES {
                    write_entry(block, pos, inode, kind, name, size as u16);
                    block[pos + size..pos + rec_len].fill(0);
                    write_header(block, pos + size, leftover as u16, 0, 0);
                } else {
                    write_entry(block, pos, inode, kind, name, rec_len as u16);
                }
                return true;
            }
        } else {
            let used = entry_size(name_len as usize).max(MIN_ENTRY_BYTES);
            let extra = rec_len as usize - used;
            if extra >= size {
                write_entry(block, pos + used, inode, kind, name, extra as u16);
                write_rec_len(block, pos, used as u16);
                return true;
            }
        }
        pos += rec_len as usize;
    }
    false
}

pub fn remove(block: &mut [u8], name: &[u8]) -> bool {
    let mut pos = 0usize;
    let mut prev: Option<usize> = None;
    while pos < BLOCK_BYTES {
        let (entry_inode, rec_len, _, _) = read_header(block, pos);
        if rec_len < MIN_ENTRY_BYTES as u16 || rec_len % 4 != 0 {
            return false;
        }
        if pos + rec_len as usize > BLOCK_BYTES {
            return false;
        }
        if entry_inode != 0 && entry_at(block, pos).name == name {
            match prev {
                Some(prev_pos) => {
                    let merged = read_rec_len(block, prev_pos) + rec_len;
                    write_rec_len(block, prev_pos, merged);
                }
                None => block[pos..pos + 4].copy_from_slice(&0u32.to_le_bytes()),
            }
            return true;
        }
        prev = Some(pos);
        pos += rec_len as usize;
    }
    false
}

pub fn live_count(block: &[u8]) -> usize {
    entries_iter(block).filter(|entry| entry.inode != 0).count()
}

fn entries_iter(block: &[u8]) -> EntryIter<'_> {
    EntryIter { block, pos: 0 }
}

struct EntryIter<'a> {
    block: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for EntryIter<'a> {
    type Item = Entry<'a>;

    fn next(&mut self) -> Option<Entry<'a>> {
        loop {
            if self.pos >= BLOCK_BYTES {
                return None;
            }
            let (inode, rec_len, _, _) = read_header(self.block, self.pos);
            if rec_len < MIN_ENTRY_BYTES as u16
                || rec_len % 4 != 0
                || self.pos + rec_len as usize > BLOCK_BYTES
            {
                return None;
            }
            let entry = entry_at(self.block, self.pos);
            self.pos += rec_len as usize;
            if inode != 0 {
                return Some(entry);
            }
        }
    }
}

fn entry_at(block: &[u8], pos: usize) -> Entry<'_> {
    let (inode, _, name_len, kind) = read_header(block, pos);
    let name = &block[pos + ENTRY_HEADER_BYTES..pos + ENTRY_HEADER_BYTES + name_len as usize];
    Entry { inode, kind, name }
}

fn read_header(block: &[u8], pos: usize) -> (u32, u16, u8, u8) {
    let inode =
        u32::from_le_bytes([block[pos], block[pos + 1], block[pos + 2], block[pos + 3]]);
    let rec_len = u16::from_le_bytes([block[pos + 4], block[pos + 5]]);
    (inode, rec_len, block[pos + 6], block[pos + 7])
}

fn read_rec_len(block: &[u8], pos: usize) -> u16 {
    u16::from_le_bytes([block[pos + 4], block[pos + 5]])
}

fn write_rec_len(block: &mut [u8], pos: usize, rec_len: u16) {
    block[pos + 4..pos + 6].copy_from_slice(&rec_len.to_le_bytes());
}

fn write_header(block: &mut [u8], pos: usize, rec_len: u16, name_len: u8, inode: u32) {
    block[pos..pos + 4].copy_from_slice(&inode.to_le_bytes());
    block[pos + 4..pos + 6].copy_from_slice(&rec_len.to_le_bytes());
    block[pos + 6] = name_len;
    block[pos + 7] = 0;
}

fn write_entry(block: &mut [u8], pos: usize, inode: u32, kind: u8, name: &[u8], rec_len: u16) {
    block[pos..pos + 4].copy_from_slice(&inode.to_le_bytes());
    block[pos + 4..pos + 6].copy_from_slice(&rec_len.to_le_bytes());
    block[pos + 6] = name.len() as u8;
    block[pos + 7] = kind;
    block[pos + ENTRY_HEADER_BYTES..pos + ENTRY_HEADER_BYTES + name.len()].copy_from_slice(name);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inode::KIND_REGULAR;
    use alloc::vec;
    use alloc::vec::Vec;

    fn empty_block() -> [u8; BLOCK_BYTES] {
        let mut block = [0u8; BLOCK_BYTES];
        init_empty(&mut block);
        block
    }

    fn dotted_block(self_inode: u32, parent: u32) -> [u8; BLOCK_BYTES] {
        let mut block = [0u8; BLOCK_BYTES];
        init_dot_entries(&mut block, self_inode, parent);
        block
    }

    #[test]
    fn entry_sizes_pad_to_four() {
        assert_eq!(entry_size(1), 12);
        assert_eq!(entry_size(4), 12);
        assert_eq!(entry_size(5), 16);
        assert_eq!(entry_size(MAX_NAME_BYTES), MAX_ENTRY_BYTES);
        assert!(MAX_ENTRY_BYTES <= BLOCK_BYTES);
    }

    #[test]
    fn empty_block_is_one_hole() {
        let block = empty_block();
        assert!(is_well_formed(&block));
        assert!(entries(&block).is_empty());
        let (inode, rec_len, name_len, kind) = read_header(&block, 0);
        assert_eq!((inode, rec_len, name_len, kind), (0, 4096, 0, 0));
    }

    #[test]
    fn zeroed_block_is_not_well_formed() {
        let block = [0u8; BLOCK_BYTES];
        assert!(!is_well_formed(&block));
    }

    #[test]
    fn dot_entries_layout_is_pinned() {
        let block = dotted_block(5, 1);
        assert!(is_well_formed(&block));
        let listed = entries(&block);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0], Entry { inode: 5, kind: KIND_DIRECTORY, name: b"." });
        assert_eq!(listed[1], Entry { inode: 1, kind: KIND_DIRECTORY, name: b".." });
        assert_eq!(read_rec_len(&block, 0), 12);
        assert_eq!(read_rec_len(&block, 12), 4084);
    }

    #[test]
    fn add_and_find_roundtrip() {
        let mut block = dotted_block(2, 1);
        assert!(add(&mut block, 9, KIND_REGULAR, b"notes.txt"));
        assert!(is_well_formed(&block));
        let found = find(&block, b"notes.txt").unwrap();
        assert_eq!((found.inode, found.kind), (9, KIND_REGULAR));
        assert!(find(&block, b"other").is_none());
    }

    #[test]
    fn lookup_is_case_sensitive_byte_equality() {
        let mut block = empty_block();
        assert!(add(&mut block, 1, KIND_REGULAR, b"Readme"));
        assert!(find(&block, b"Readme").is_some());
        assert!(find(&block, b"readme").is_none());
        assert!(find(&block, b"README").is_none());
        assert!(find(&block, b"Readm").is_none());
        assert!(find(&block, b"Readme ").is_none());
    }

    #[test]
    fn add_carves_from_live_entry_tail_and_keeps_tiling() {
        let mut block = dotted_block(2, 1);
        for name in [&b"alpha"[..], &b"beta"[..], &b"gamma"[..]] {
            assert!(add(&mut block, 10, KIND_REGULAR, name));
        }
        assert!(is_well_formed(&block));
        assert_eq!(entries(&block).len(), 5);
    }

    #[test]
    fn remove_absorbs_into_previous_and_readd_succeeds() {
        let mut block = dotted_block(2, 1);
        assert!(add(&mut block, 7, KIND_REGULAR, b"one"));
        assert!(add(&mut block, 8, KIND_REGULAR, b"two"));
        assert!(add(&mut block, 9, KIND_REGULAR, b"three"));
        assert!(remove(&mut block, b"two"));
        assert!(is_well_formed(&block));
        assert!(find(&block, b"two").is_none());
        assert!(find(&block, b"three").is_some());
        let after = find(&block, b"one").unwrap();
        assert_eq!(after.inode, 7);
        assert!(add(&mut block, 11, KIND_REGULAR, b"reused"));
        assert_eq!(entries(&block).len(), 5);
    }

    #[test]
    fn remove_first_entry_leaves_hole_that_add_reuses() {
        let mut block = dotted_block(2, 1);
        assert!(remove(&mut block, b"."));
        let (inode, rec_len, _, _) = read_header(&block, 0);
        assert_eq!((inode, rec_len), (0, 12));
        assert!(add(&mut block, 20, KIND_REGULAR, b"x"));
        assert_eq!(find(&block, b"x").unwrap().inode, 20);
        assert!(is_well_formed(&block));
    }

    #[test]
    fn hole_splits_only_when_leftover_fits_another_entry() {
        let mut block = dotted_block(2, 1);
        assert!(remove(&mut block, b"."));
        let mut name_255 = vec![b'z'; MAX_NAME_BYTES];
        name_255[0] = b'z';
        assert!(add(&mut block, 30, KIND_REGULAR, &name_255));
        assert!(is_well_formed(&block));
        assert_eq!(find(&block, &name_255).unwrap().inode, 30);
        assert_eq!(find(&block, b"..").unwrap().inode, 1);
    }

    #[test]
    fn add_rejects_bad_names_and_zero_inode() {
        let mut block = dotted_block(2, 1);
        assert!(!add(&mut block, 5, KIND_REGULAR, b""));
        assert!(!add(&mut block, 5, KIND_REGULAR, &vec![b'q'; 256]));
        assert!(!add(&mut block, 0, KIND_REGULAR, b"ok"));
        assert!(add(&mut block, 5, KIND_REGULAR, &vec![b'q'; 255]));
    }

    #[test]
    fn fill_then_free_a_slot() {
        let mut block = dotted_block(1, 1);
        let mut names: Vec<Vec<u8>> = Vec::new();
        let mut added = 0usize;
        for i in 0..1000u32 {
            let name = alloc::format!("{i:04}");
            if add(&mut block, i + 2, KIND_REGULAR, name.as_bytes()) {
                names.push(name.into_bytes());
                added += 1;
            } else {
                break;
            }
        }
        assert!((250..1000).contains(&added));
        assert!(is_well_formed(&block));
        assert!(!add(&mut block, 5000, KIND_REGULAR, b"free"));
        let doomed = &names[added / 2];
        assert!(remove(&mut block, doomed));
        assert!(add(&mut block, 5000, KIND_REGULAR, b"free"));
        assert!(is_well_formed(&block));
    }

    #[test]
    fn entries_list_order_is_block_order() {
        let mut block = empty_block();
        assert!(add(&mut block, 1, KIND_REGULAR, b"aaa"));
        assert!(add(&mut block, 2, KIND_REGULAR, b"bbb"));
        assert!(add(&mut block, 3, KIND_REGULAR, b"ccc"));
        let names: Vec<&[u8]> = entries(&block).iter().map(|e| e.name).collect();
        assert_eq!(names, vec![&b"aaa"[..], &b"bbb"[..], &b"ccc"[..]]);
    }

    #[test]
    fn well_formed_rejects_corruption() {
        let mut block = dotted_block(1, 1);
        block[4] = 6;
        assert!(!is_well_formed(&block));

        let mut block = dotted_block(1, 1);
        block[5] = 0x11;
        assert!(!is_well_formed(&block));

        let mut block = dotted_block(1, 1);
        block[7] = 9;
        assert!(!is_well_formed(&block));

        let mut block = dotted_block(1, 1);
        block[6] = 0;
        assert!(!is_well_formed(&block));

        let mut block = dotted_block(1, 1);
        write_rec_len(&mut block, 12, 4096);
        assert!(!is_well_formed(&block));
    }

    #[test]
    fn live_count_ignores_holes() {
        let mut block = dotted_block(2, 1);
        assert_eq!(live_count(&block), 2);
        assert!(add(&mut block, 9, KIND_REGULAR, b"file"));
        assert_eq!(live_count(&block), 3);
        assert!(remove(&mut block, b"."));
        assert_eq!(live_count(&block), 2);
    }
}
