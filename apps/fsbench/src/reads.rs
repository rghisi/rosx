use crate::seed::BIG_FILE_BYTES;
use crate::windows;
use alloc::format;
use usrlib::println;
use vfs_protocol::client::Vfs;

const ELF_PATH: &str = "/snake";
const SMALL_READ_BYTES: usize = 512;
const CHUNK_READ_BYTES: usize = 4096;

pub fn run(vfs: &Vfs, big_path: &str) {
    read_full(vfs, big_path);
    read_small(vfs, big_path);
    elf_load(vfs);
}

fn read_full(vfs: &Vfs, path: &str) {
    let detail = format!("kind=whole bytes={BIG_FILE_BYTES} chunk={CHUNK_READ_BYTES}");
    windows::run_phase("fsbench_read_full", &detail, &mut || {
        read_via_chunks(vfs, path)
    });
}

fn read_via_chunks(vfs: &Vfs, path: &str) -> bool {
    let Ok(mut file) = vfs.open(path, 0) else {
        return false;
    };
    let mut buffer = [0u8; CHUNK_READ_BYTES];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => {
                let _ = file.close();
                return false;
            }
        }
    }
    file.close().is_ok()
}

fn read_small(vfs: &Vfs, path: &str) {
    let Ok(mut file) = vfs.open(path, 0) else {
        println!("bench.fsbench_read_small kind=sequential bytes={SMALL_READ_BYTES} result=fail stage=open");
        return;
    };
    let detail = format!("kind=sequential bytes={SMALL_READ_BYTES}");
    windows::run_phase("fsbench_read_small", &detail, &mut || {
        let mut buffer = [0u8; SMALL_READ_BYTES];
        match file.read(&mut buffer) {
            Ok(SMALL_READ_BYTES) => true,
            Ok(0) => {
                file.seek(0);
                true
            }
            Ok(_) => false,
            Err(_) => false,
        }
    });
    let _ = file.close();
}

fn elf_load(vfs: &Vfs) {
    let detail = format!("kind=read_whole file={ELF_PATH}");
    windows::run_phase("fsbench_elf_load", &detail, &mut || {
        vfs.read_whole(ELF_PATH).is_ok()
    });
}
