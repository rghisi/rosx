use std::path::{Path, PathBuf};

pub fn create_disk_image(kernel_elf: &Path, dest_dir: &Path) -> PathBuf {
    let img = dest_dir.join("rosx.img");
    match bootloader::BiosBoot::new(kernel_elf).create_disk_image(&img) {
        Ok(()) => img,
        Err(err) => panic!(
            "failed to create disk image '{}' from kernel '{}': {err:#}",
            img.display(),
            kernel_elf.display()
        ),
    }
}
