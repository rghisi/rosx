use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use vfs_protocol::wire::{self, VfsError};

pub const MODE_WRITE: u8 = 1;
pub const MODE_CREATE: u8 = 2;

pub struct OpenEntry {
    pub owner: u32,
    pub mode: u8,
    pub name: String,
}

pub struct RamFs {
    files: BTreeMap<String, Vec<u8>>,
    opens: BTreeMap<u32, OpenEntry>,
    next_handle: u32,
}

impl RamFs {
    pub fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            opens: BTreeMap::new(),
            next_handle: 1,
        }
    }

    pub fn open(&mut self, owner: u32, mode: u8, name: &[u8]) -> Result<u32, VfsError> {
        let name = validate_name(name)?;
        if self.files.contains_key(name) {
            if mode & MODE_CREATE != 0 {
                return Err(VfsError::AlreadyExists);
            }
        } else if mode & MODE_CREATE == 0 {
            return Err(VfsError::NotFound);
        } else {
            self.files.insert(String::from(name), Vec::new());
        }
        let handle = self.next_handle;
        self.next_handle += 1;
        self.opens.insert(
            handle,
            OpenEntry {
                owner,
                mode,
                name: String::from(name),
            },
        );
        Ok(handle)
    }

    pub fn read(
        &self,
        owner: u32,
        handle: u32,
        offset: u64,
        count: u32,
    ) -> Result<Vec<u8>, VfsError> {
        let file = self.live_file(owner, handle)?;
        let start = offset.min(file.len() as u64) as usize;
        let end = start.saturating_add(count as usize).min(file.len());
        Ok(file[start..end].to_vec())
    }

    pub fn write(
        &mut self,
        owner: u32,
        handle: u32,
        offset: u64,
        payload: &[u8],
    ) -> Result<usize, VfsError> {
        let entry = match self.opens.get(&handle) {
            Some(entry) if entry.owner == owner => entry,
            _ => return Err(VfsError::BadHandle),
        };
        let Some(file) = self.files.get_mut(&entry.name) else {
            return Err(VfsError::NotFound);
        };
        if payload.is_empty() {
            return Ok(0);
        }
        if entry.mode & MODE_WRITE == 0 {
            return Err(VfsError::NotPermitted);
        }
        let offset = offset as usize;
        let end = offset.saturating_add(payload.len());
        if file.len() < end {
            file.resize(end, 0);
        }
        file[offset..end].copy_from_slice(payload);
        Ok(payload.len())
    }

    pub fn close(&mut self, owner: u32, handle: u32) -> Result<(), VfsError> {
        match self.opens.get(&handle) {
            Some(entry) if entry.owner == owner => {
                self.opens.remove(&handle);
                Ok(())
            }
            _ => Err(VfsError::BadHandle),
        }
    }

    pub fn delete(&mut self, name: &[u8]) -> Result<(), VfsError> {
        let name = validate_name(name)?;
        self.files
            .remove(name)
            .map(|_| ())
            .ok_or(VfsError::NotFound)
    }

    pub fn list(&self) -> Vec<u8> {
        let mut blob = Vec::new();
        for (index, name) in self.files.keys().enumerate() {
            if index > 0 {
                blob.push(b'\n');
            }
            blob.extend_from_slice(name.as_bytes());
        }
        blob
    }

    fn live_file(&self, owner: u32, handle: u32) -> Result<&Vec<u8>, VfsError> {
        let entry = match self.opens.get(&handle) {
            Some(entry) if entry.owner == owner => entry,
            _ => return Err(VfsError::BadHandle),
        };
        self.files.get(&entry.name).ok_or(VfsError::NotFound)
    }
}

fn validate_name(name: &[u8]) -> Result<&str, VfsError> {
    let Ok(text) = core::str::from_utf8(name) else {
        return Err(VfsError::InvalidPath);
    };
    if wire::valid_name(text) {
        Ok(text)
    } else {
        Err(VfsError::InvalidPath)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: u32 = 100;
    const OTHER: u32 = 200;

    fn opened(store: &mut RamFs, owner: u32, name: &str, mode: u8) -> u32 {
        store
            .open(owner, mode, name.as_bytes())
            .expect("open succeeds")
    }

    fn write_all(store: &mut RamFs, owner: u32, handle: u32, offset: u64, payload: &[u8]) {
        let written = store
            .write(owner, handle, offset, payload)
            .expect("write succeeds");
        assert_eq!(written, payload.len());
    }

    fn contents(store: &RamFs, owner: u32, handle: u32) -> Vec<u8> {
        store.read(owner, handle, 0, 65_536).expect("read succeeds")
    }

    #[test]
    fn open_rule_create_absent_creates_empty_file_and_opens() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        assert_eq!(store.list(), b"notes.txt");
        assert_eq!(contents(&store, OWNER, handle), Vec::<u8>::new());
    }

    #[test]
    fn open_rule_existing_without_create_keeps_content_and_requested_mode() {
        let mut store = RamFs::new();
        let first = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        write_all(&mut store, OWNER, first, 0, b"kept");
        store.close(OWNER, first).expect("close succeeds");
        let second = opened(&mut store, OWNER, "notes.txt", 0);
        assert_eq!(contents(&store, OWNER, second), b"kept");
        assert_eq!(
            store.write(OWNER, second, 0, b"no"),
            Err(VfsError::NotPermitted)
        );
    }

    #[test]
    fn open_rule_existing_with_create_is_already_exists() {
        let mut store = RamFs::new();
        opened(&mut store, OWNER, "notes.txt", MODE_CREATE);
        assert_eq!(
            store.open(OWNER, MODE_CREATE, b"notes.txt"),
            Err(VfsError::AlreadyExists)
        );
    }

    #[test]
    fn open_rule_absent_without_create_is_not_found() {
        let mut store = RamFs::new();
        assert_eq!(
            store.open(OWNER, 0, b"missing.txt"),
            Err(VfsError::NotFound)
        );
    }

    #[test]
    fn open_ignores_unknown_mode_bits() {
        let mut store = RamFs::new();
        let seed = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        write_all(&mut store, OWNER, seed, 0, b"abc");
        let exotic = opened(&mut store, OWNER, "notes.txt", 0x08);
        assert_eq!(contents(&store, OWNER, exotic), b"abc");
        assert_eq!(
            store.write(OWNER, exotic, 0, b"x"),
            Err(VfsError::NotPermitted)
        );
        assert_eq!(
            store.open(OWNER, 0x10, b"absent.txt"),
            Err(VfsError::NotFound)
        );
    }

    #[test]
    fn handles_start_at_one_and_increase_monotonically() {
        let mut store = RamFs::new();
        assert_eq!(opened(&mut store, OWNER, "a", MODE_CREATE), 1);
        assert_eq!(opened(&mut store, OWNER, "b", MODE_CREATE), 2);
        assert_eq!(opened(&mut store, OTHER, "a", 0), 3);
    }

    #[test]
    fn open_rejects_empty_name_via_shared_validator() {
        let mut store = RamFs::new();
        assert_eq!(
            store.open(OWNER, MODE_CREATE, b""),
            Err(VfsError::InvalidPath)
        );
    }

    #[test]
    fn open_accepts_name_of_fifty_seven_bytes() {
        let mut store = RamFs::new();
        let name = alloc::vec![b'x'; 57];
        let handle = opened(
            &mut store,
            OWNER,
            core::str::from_utf8(&name).unwrap(),
            MODE_CREATE,
        );
        assert_eq!(store.list(), name);
        assert_eq!(contents(&store, OWNER, handle), Vec::<u8>::new());
    }

    #[test]
    fn open_rejects_name_of_fifty_eight_bytes() {
        let mut store = RamFs::new();
        let name = alloc::vec![b'x'; 58];
        assert_eq!(
            store.open(OWNER, MODE_CREATE, &name),
            Err(VfsError::InvalidPath)
        );
    }

    #[test]
    fn open_rejects_name_containing_newline_byte() {
        let mut store = RamFs::new();
        assert_eq!(
            store.open(OWNER, MODE_CREATE, b"bad\nname"),
            Err(VfsError::InvalidPath)
        );
    }

    #[test]
    fn open_rejects_name_containing_nul_byte() {
        let mut store = RamFs::new();
        assert_eq!(
            store.open(OWNER, MODE_CREATE, b"bad\0name"),
            Err(VfsError::InvalidPath)
        );
    }

    #[test]
    fn delete_rejects_invalid_names_via_shared_validator() {
        let mut store = RamFs::new();
        assert_eq!(store.delete(b""), Err(VfsError::InvalidPath));
        assert_eq!(store.delete(b"bad\nname"), Err(VfsError::InvalidPath));
        assert_eq!(store.delete(b"bad\0name"), Err(VfsError::InvalidPath));
        let name = alloc::vec![b'x'; 58];
        assert_eq!(store.delete(&name), Err(VfsError::InvalidPath));
    }

    #[test]
    fn read_on_empty_file_returns_empty() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_CREATE);
        assert_eq!(contents(&store, OWNER, handle), Vec::<u8>::new());
        assert_eq!(
            store.read(OWNER, handle, 0, 100).expect("read succeeds"),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn read_at_and_past_end_returns_empty() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        write_all(&mut store, OWNER, handle, 0, b"0123456789");
        assert_eq!(
            store.read(OWNER, handle, 10, 4).expect("read succeeds"),
            Vec::<u8>::new()
        );
        assert_eq!(
            store.read(OWNER, handle, 1_000, 4).expect("read succeeds"),
            Vec::<u8>::new()
        );
        assert_eq!(
            store
                .read(OWNER, handle, u64::MAX, 4)
                .expect("read succeeds"),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn read_clamps_count_across_end() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        write_all(&mut store, OWNER, handle, 0, b"0123456789");
        assert_eq!(
            store.read(OWNER, handle, 5, 100).expect("read succeeds"),
            b"56789"
        );
        assert_eq!(
            store.read(OWNER, handle, 2, 3).expect("read succeeds"),
            b"234"
        );
    }

    #[test]
    fn write_without_write_mode_is_not_permitted() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_CREATE);
        assert_eq!(
            store.write(OWNER, handle, 0, b"denied"),
            Err(VfsError::NotPermitted)
        );
        assert_eq!(contents(&store, OWNER, handle), Vec::<u8>::new());
    }

    #[test]
    fn read_on_read_only_handle_is_permitted() {
        let mut store = RamFs::new();
        let seed = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        write_all(&mut store, OWNER, seed, 0, b"readonly ok");
        store.close(OWNER, seed).expect("close succeeds");
        let reader = opened(&mut store, OWNER, "notes.txt", 0);
        assert_eq!(contents(&store, OWNER, reader), b"readonly ok");
    }

    #[test]
    fn write_at_zero_into_empty_file() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        write_all(&mut store, OWNER, handle, 0, b"hello");
        assert_eq!(contents(&store, OWNER, handle), b"hello");
    }

    #[test]
    fn write_appends_at_end() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        write_all(&mut store, OWNER, handle, 0, b"hello");
        write_all(&mut store, OWNER, handle, 5, b" world");
        assert_eq!(contents(&store, OWNER, handle), b"hello world");
    }

    #[test]
    fn write_beyond_end_zero_fills_hole_byte_exactly() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        write_all(&mut store, OWNER, handle, 0, b"abc");
        write_all(&mut store, OWNER, handle, 6, b"xy");
        assert_eq!(contents(&store, OWNER, handle), b"abc\0\0\0xy");
        write_all(&mut store, OWNER, handle, 1, b"Z");
        assert_eq!(contents(&store, OWNER, handle), b"aZc\0\0\0xy");
    }

    #[test]
    fn empty_write_is_noop_and_never_resizes() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        write_all(&mut store, OWNER, handle, 0, b"abc");
        assert_eq!(store.write(OWNER, handle, 0, b"").expect("ok"), 0);
        assert_eq!(store.write(OWNER, handle, 100, b"").expect("ok"), 0);
        assert_eq!(contents(&store, OWNER, handle), b"abc");
        let reader = opened(&mut store, OWNER, "notes.txt", 0);
        assert_eq!(store.write(OWNER, reader, 0, b"").expect("ok"), 0);
        assert_eq!(contents(&store, OWNER, reader), b"abc");
    }

    #[test]
    fn close_frees_handle_and_second_close_is_bad_handle() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_CREATE);
        store.close(OWNER, handle).expect("close succeeds");
        assert_eq!(store.close(OWNER, handle), Err(VfsError::BadHandle));
        assert_eq!(store.read(OWNER, handle, 0, 10), Err(VfsError::BadHandle));
    }

    #[test]
    fn cross_owner_handle_use_is_bad_handle() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        write_all(&mut store, OWNER, handle, 0, b"secret");
        assert_eq!(store.read(OTHER, handle, 0, 10), Err(VfsError::BadHandle));
        assert_eq!(
            store.write(OTHER, handle, 0, b"evil"),
            Err(VfsError::BadHandle)
        );
        assert_eq!(store.close(OTHER, handle), Err(VfsError::BadHandle));
        assert_eq!(contents(&store, OWNER, handle), b"secret");
    }

    #[test]
    fn unknown_handle_is_bad_handle_for_every_op() {
        let mut store = RamFs::new();
        assert_eq!(store.read(OWNER, 0, 0, 10), Err(VfsError::BadHandle));
        assert_eq!(store.read(OWNER, 99, 0, 10), Err(VfsError::BadHandle));
        assert_eq!(store.write(OWNER, 99, 0, b"x"), Err(VfsError::BadHandle));
        assert_eq!(store.close(OWNER, 99), Err(VfsError::BadHandle));
    }

    #[test]
    fn delete_then_read_on_open_handle_is_not_found() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        write_all(&mut store, OWNER, handle, 0, b"doomed");
        store.delete(b"notes.txt").expect("delete succeeds");
        assert_eq!(store.read(OWNER, handle, 0, 10), Err(VfsError::NotFound));
    }

    #[test]
    fn delete_then_write_on_open_handle_is_not_found() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        store.delete(b"notes.txt").expect("delete succeeds");
        assert_eq!(
            store.write(OWNER, handle, 0, b"ghost"),
            Err(VfsError::NotFound)
        );
        assert_eq!(store.write(OWNER, handle, 0, b""), Err(VfsError::NotFound));
    }

    #[test]
    fn delete_recreates_clean_file_after_reopen() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        write_all(&mut store, OWNER, handle, 0, b"old");
        store.close(OWNER, handle).expect("close succeeds");
        store.delete(b"notes.txt").expect("delete succeeds");
        let fresh = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        assert_eq!(contents(&store, OWNER, fresh), Vec::<u8>::new());
    }

    #[test]
    fn close_after_delete_is_ok() {
        let mut store = RamFs::new();
        let handle = opened(&mut store, OWNER, "notes.txt", MODE_CREATE);
        store.delete(b"notes.txt").expect("delete succeeds");
        store.close(OWNER, handle).expect("close succeeds");
        assert_eq!(store.close(OWNER, handle), Err(VfsError::BadHandle));
    }

    #[test]
    fn delete_missing_name_is_not_found() {
        let mut store = RamFs::new();
        assert_eq!(store.delete(b"missing.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn list_joins_sorted_names_with_newline() {
        let mut store = RamFs::new();
        opened(&mut store, OWNER, "notes.txt", MODE_CREATE);
        opened(&mut store, OWNER, "blob.bin", MODE_CREATE);
        assert_eq!(store.list(), b"blob.bin\nnotes.txt");
    }

    #[test]
    fn list_empty_store_is_empty() {
        let store = RamFs::new();
        assert!(store.list().is_empty());
    }

    #[test]
    fn two_handles_on_one_file_share_content() {
        let mut store = RamFs::new();
        let first = opened(&mut store, OWNER, "notes.txt", MODE_WRITE | MODE_CREATE);
        let second = opened(&mut store, OWNER, "notes.txt", MODE_WRITE);
        write_all(&mut store, OWNER, first, 0, b"shared");
        assert_eq!(contents(&store, OWNER, second), b"shared");
        write_all(&mut store, OWNER, second, 0, b"S");
        assert_eq!(contents(&store, OWNER, first), b"Shared");
    }
}
