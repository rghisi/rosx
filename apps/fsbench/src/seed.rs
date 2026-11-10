use alloc::vec::Vec;
use usrlib::println;
use vfs_protocol::client::{Vfs, MODE_CREATE, MODE_WRITE};

pub(crate) const BIG_FILE_BYTES: usize = 1024 * 1024;
const WRITE_CHUNK_BYTES: usize = 4096;

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
