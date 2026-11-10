use crate::seed::wide_names;
use crate::windows;
use alloc::format;
use vfs_protocol::client::Vfs;

pub fn run(vfs: &Vfs, wide_path: &str) {
    let names = wide_names(wide_path);
    lookup_hit(vfs, &names);
    stat_path(vfs, &names);
    readdir(vfs, wide_path, names.len());
}

fn lookup_hit(vfs: &Vfs, names: &[alloc::string::String]) {
    let detail = format!("kind=open_close entries={}", names.len());
    let mut cursor = 0usize;
    windows::run_phase("fsbench_lookup_hit", &detail, &mut || {
        let path = &names[cursor];
        cursor = (cursor + 1) % names.len();
        let Ok(file) = vfs.open(path, 0) else {
            return false;
        };
        file.close().is_ok()
    });
}

fn stat_path(vfs: &Vfs, names: &[alloc::string::String]) {
    let detail = format!("kind=stat entries={}", names.len());
    let mut cursor = 0usize;
    windows::run_phase("fsbench_stat_path", &detail, &mut || {
        let path = &names[cursor];
        cursor = (cursor + 1) % names.len();
        vfs.stat(path).is_ok()
    });
}

fn readdir(vfs: &Vfs, wide_path: &str, entries: usize) {
    let detail = format!("kind=list entries={entries}");
    windows::run_phase("fsbench_readdir", &detail, &mut || {
        matches!(vfs.list(wide_path), Ok(items) if !items.is_empty())
    });
}
