use std::fmt;

pub const SECTOR_BYTES: u64 = 512;
pub const MAX_SECTORS_PER_CLUSTER: u32 = 128;

const DIR_ENTRY_BYTES: u64 = 32;
const FAT16_RESERVED_SECTORS: u64 = 1;
const FAT16_ROOT_ENTRIES: u64 = 512;
const FAT32_RESERVED_SECTORS: u64 = 32;
const FAT16_MIN_CLUSTERS: u64 = 4_086;
const FAT16_CLUSTER_LIMIT: u64 = 65_525;
const FAT32_MIN_CLUSTERS: u64 = 65_525;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatKind {
    Fat16,
    Fat32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryError {
    CapacityTooSmall(FatKind),
    NoClusterSize(FatKind),
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeometryError::CapacityTooSmall(kind) => {
                write!(f, "capacity too small for a {kind:?} volume")
            }
            GeometryError::NoClusterSize(kind) => {
                write!(f, "no cluster size keeps this volume inside the {kind:?} cluster range")
            }
        }
    }
}

impl std::error::Error for GeometryError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeGeometry {
    pub kind: FatKind,
    pub total_sectors: u64,
    pub sectors_per_cluster: u32,
    pub reserved_sectors: u64,
    pub fat_sectors: u64,
    pub root_entries: u64,
    pub cluster_count: u64,
    pub first_data_sector: u64,
}

impl VolumeGeometry {
    pub fn root_dir_sectors(&self) -> u64 {
        self.root_entries * DIR_ENTRY_BYTES / SECTOR_BYTES
    }

    pub fn cluster_bytes(&self) -> u64 {
        self.sectors_per_cluster as u64 * SECTOR_BYTES
    }

    pub fn first_sector_of_cluster(&self, cluster: u32) -> Option<u64> {
        if cluster < 2 {
            return None;
        }
        let index = u64::from(cluster) - 2;
        if index >= self.cluster_count {
            return None;
        }
        Some(self.first_data_sector + index * self.sectors_per_cluster as u64)
    }
}

pub fn plan_volume(kind: FatKind, capacity_bytes: u64) -> Result<VolumeGeometry, GeometryError> {
    let total = capacity_bytes / SECTOR_BYTES;
    let (reserved, root_entries) = match kind {
        FatKind::Fat16 => (FAT16_RESERVED_SECTORS, FAT16_ROOT_ENTRIES),
        FatKind::Fat32 => (FAT32_RESERVED_SECTORS, 0),
    };
    let root_dir_sectors = root_entries * DIR_ENTRY_BYTES / SECTOR_BYTES;
    let Some(usable) = total
        .checked_sub(reserved + root_dir_sectors)
        .filter(|usable| *usable > 0)
    else {
        return Err(GeometryError::CapacityTooSmall(kind));
    };
    let mut spc = 1u32;
    loop {
        let upper_clusters = usable / u64::from(spc);
        let fat = fat_sectors_for(kind, upper_clusters);
        if let Some(slack) = usable.checked_sub(2 * fat) {
            let clusters = slack / u64::from(spc);
            if fat_sectors_for(kind, clusters) <= fat && clusters_in_range(kind, clusters) {
                return Ok(VolumeGeometry {
                    kind,
                    total_sectors: total,
                    sectors_per_cluster: spc,
                    reserved_sectors: reserved,
                    fat_sectors: fat,
                    root_entries,
                    cluster_count: clusters,
                    first_data_sector: reserved + 2 * fat + root_dir_sectors,
                });
            }
        }
        if clusters_above_range(kind, upper_clusters) {
            if spc >= MAX_SECTORS_PER_CLUSTER {
                return Err(GeometryError::NoClusterSize(kind));
            }
            spc *= 2;
        } else {
            return Err(GeometryError::CapacityTooSmall(kind));
        }
    }
}

fn fat_sectors_for(kind: FatKind, clusters: u64) -> u64 {
    let entry_bytes = match kind {
        FatKind::Fat16 => 2,
        FatKind::Fat32 => 4,
    };
    let entries = clusters + 2;
    (entries * entry_bytes).div_ceil(SECTOR_BYTES)
}

fn clusters_in_range(kind: FatKind, clusters: u64) -> bool {
    match kind {
        FatKind::Fat16 => (FAT16_MIN_CLUSTERS..FAT16_CLUSTER_LIMIT).contains(&clusters),
        FatKind::Fat32 => clusters >= FAT32_MIN_CLUSTERS,
    }
}

fn clusters_above_range(kind: FatKind, clusters: u64) -> bool {
    match kind {
        FatKind::Fat16 => clusters >= FAT16_CLUSTER_LIMIT,
        FatKind::Fat32 => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn plans_fat16_at_one_sector_per_cluster() {
        let geometry = plan_volume(FatKind::Fat16, 8 * MIB).unwrap();
        assert_eq!(geometry.sectors_per_cluster, 1);
        assert_eq!(geometry.reserved_sectors, 1);
        assert_eq!(geometry.root_entries, 512);
        assert!((FAT16_MIN_CLUSTERS..FAT16_CLUSTER_LIMIT).contains(&geometry.cluster_count));
        assert_eq!(
            geometry.first_data_sector,
            geometry.reserved_sectors + 2 * geometry.fat_sectors + geometry.root_dir_sectors()
        );
        assert!(geometry.total_sectors >= geometry.first_data_sector + geometry.cluster_count);
    }

    #[test]
    fn plans_fat16_growing_clusters_before_giving_up() {
        let geometry = plan_volume(FatKind::Fat16, 64 * MIB).unwrap();
        assert!(geometry.sectors_per_cluster > 1);
        assert!(geometry.cluster_count < FAT16_CLUSTER_LIMIT);
    }

    #[test]
    fn rejects_fat16_below_the_cluster_floor_and_above_the_ceiling() {
        assert_eq!(
            plan_volume(FatKind::Fat16, MIB),
            Err(GeometryError::CapacityTooSmall(FatKind::Fat16))
        );
        assert_eq!(
            plan_volume(FatKind::Fat16, 512 * 1024 * MIB),
            Err(GeometryError::NoClusterSize(FatKind::Fat16))
        );
    }

    #[test]
    fn plans_fat32_only_at_or_above_its_cluster_floor() {
        let geometry = plan_volume(FatKind::Fat32, 64 * MIB).unwrap();
        assert_eq!(geometry.sectors_per_cluster, 1);
        assert_eq!(geometry.reserved_sectors, 32);
        assert_eq!(geometry.root_entries, 0);
        assert!(geometry.cluster_count >= FAT32_MIN_CLUSTERS);
        assert_eq!(
            plan_volume(FatKind::Fat32, 8 * MIB),
            Err(GeometryError::CapacityTooSmall(FatKind::Fat32))
        );
    }

    #[test]
    fn cluster_mapping_matches_the_sector_window() {
        let geometry = plan_volume(FatKind::Fat16, 8 * MIB).unwrap();
        assert_eq!(geometry.first_sector_of_cluster(0), None);
        assert_eq!(geometry.first_sector_of_cluster(1), None);
        assert_eq!(
            geometry.first_sector_of_cluster(2),
            Some(geometry.first_data_sector)
        );
        assert_eq!(
            geometry.first_sector_of_cluster(geometry.cluster_count as u32 + 1),
            Some(geometry.first_data_sector + geometry.cluster_count - 1)
        );
        assert_eq!(geometry.first_sector_of_cluster(geometry.cluster_count as u32 + 2), None);
    }
}
