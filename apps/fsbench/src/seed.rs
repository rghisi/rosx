use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use usrlib::println;
use vfs_protocol::client::{Vfs, MODE_CREATE, MODE_WRITE};

pub(crate) const BIG_FILE_BYTES: usize = 1024 * 1024;
pub(crate) const WIDE_ENTRY_COUNT: usize = 150;
const WRITE_CHUNK_BYTES: usize = 4096;

pub(crate) fn wide_names(wide_path: &str) -> Vec<String> {
    (0..WIDE_ENTRY_COUNT)
        .map(|index| format!("{wide_path}/f{index:03}.dat"))
        .collect()
}

pub(crate) fn ensure_wide_dir(vfs: &Vfs, wide_path: &str) -> bool {
    for path in wide_names(wide_path) {
        let _ = vfs.delete(&path);
    }
    let _ = vfs.rmdir(wide_path);
    let _ = vfs.mkdir(wide_path);
    for path in wide_names(wide_path) {
        let Ok(file) = vfs.open(&path, MODE_CREATE | MODE_WRITE) else {
            println!("fsbench seed {wide_path} create failed at {path}");
            return false;
        };
        if file.close().is_err() {
            println!("fsbench seed {wide_path} close failed at {path}");
            return false;
        }
    }
    println!("fsbench seed {wide_path} entries={WIDE_ENTRY_COUNT} ok");
    true
}

pub(crate) fn lcg_fill(len: usize) -> Vec<u8> {
    let mut state: u32 = 0x9E3779B9;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state & 0xFF) as u8
        })
        .collect()
}

pub(crate) fn ensure_big_file(vfs: &Vfs, path: &str) -> bool {
    let _ = vfs.delete(path);
    let mut file = match vfs.open(path, MODE_CREATE | MODE_WRITE) {
        Ok(file) => file,
        Err(error) => {
            println!("fsbench seed {path} open failed ({error:?})");
            return false;
        }
    };
    let chunk = lcg_fill(WRITE_CHUNK_BYTES);
    let mut written = 0usize;
    while written < BIG_FILE_BYTES {
        if let Err(error) = file.write(&chunk) {
            println!("fsbench seed {path} write failed at {written} ({error:?})");
            return false;
        }
        written += WRITE_CHUNK_BYTES;
    }
    if let Err(error) = file.close() {
        println!("fsbench seed {path} close failed ({error:?})");
        return false;
    }
    println!("fsbench seed {path} bytes={BIG_FILE_BYTES} ok");
    true
}
