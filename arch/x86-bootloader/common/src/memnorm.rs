use system::boot_info::MEMORY_MAP_MAX_ENTRIES;
use system::boot_info::MemoryKind;
use system::boot_info::MemoryRegion;

pub const FIRST_MEGABYTE: u64 = 0x10_0000;
pub const SIXTEEN_MEGABYTES: u64 = 0x100_0000;
pub const E801_BLOCK_SENTINEL: u16 = 0x1C20;
pub const E88_CAP_BYTES: u64 = 63 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RawEntry {
    pub base: u64,
    pub length: u64,
    pub kind: u32,
}

impl RawEntry {
    pub const fn usable(base: u64, length: u64) -> Self {
        Self {
            base,
            length,
            kind: 1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemNormError {
    Empty,
    RawOverlap,
    Overflow,
}

const RESERVATIONS: &[(u64, u64, MemoryKind)] = &[
    (0x0000_0000, 0x0000_0400, MemoryKind::Reserved),
    (0x0000_0400, 0x0000_0500, MemoryKind::Reserved),
    (0x0000_0500, 0x0000_0600, MemoryKind::BootloaderReclaimable),
    (0x0000_0600, 0x0000_0800, MemoryKind::BootloaderReclaimable),
    (0x0000_0800, 0x0000_7C00, MemoryKind::BootloaderReclaimable),
    (0x0000_7C00, 0x0000_7E00, MemoryKind::BootloaderReclaimable),
    (0x0000_7E00, 0x0003_0000, MemoryKind::BootloaderReclaimable),
    (0x0003_0000, 0x0003_1000, MemoryKind::Reserved),
    (0x0003_1000, 0x0004_0000, MemoryKind::Usable),
    (0x0004_0000, 0x0007_C000, MemoryKind::BootloaderReclaimable),
    (0x0007_C000, 0x0008_0000, MemoryKind::Reserved),
    (0x0008_0000, FIRST_MEGABYTE, MemoryKind::Reserved),
];

pub fn synthesize_e801(ax: u16, cx: u16) -> ([RawEntry; 3], usize) {
    let one_to_sixteen = if ax == E801_BLOCK_SENTINEL {
        u64::from(ax) * 64 * 1024
    } else {
        u64::from(ax) * 1024
    };
    let mut entries = [RawEntry::usable(0, 0); 3];
    entries[0] = RawEntry::usable(0, FIRST_MEGABYTE);
    entries[1] = RawEntry::usable(FIRST_MEGABYTE, one_to_sixteen);
    let mut count = 2;
    if cx != 0 {
        entries[2] = RawEntry::usable(SIXTEEN_MEGABYTES, u64::from(cx) * 1024);
        count = 3;
    }
    (entries, count)
}

pub fn synthesize_e88(kib: u16) -> ([RawEntry; 2], usize) {
    let above_one_megabyte = (u64::from(kib) * 1024).min(E88_CAP_BYTES);
    let mut entries = [RawEntry::usable(0, 0); 2];
    entries[0] = RawEntry::usable(0, FIRST_MEGABYTE);
    entries[1] = RawEntry::usable(FIRST_MEGABYTE, above_one_megabyte);
    (entries, 2)
}

pub fn normalize(
    raw: &[RawEntry],
    kernel_carve_out: Option<(u64, u64)>,
    out: &mut [MemoryRegion],
) -> Result<usize, MemNormError> {
    let mut mapped = [zero_region(); MEMORY_MAP_MAX_ENTRIES];
    let mut count = 0usize;
    for entry in raw {
        if entry.length == 0 {
            continue;
        }
        if count == MEMORY_MAP_MAX_ENTRIES {
            return Err(MemNormError::Overflow);
        }
        mapped[count] = MemoryRegion {
            base: entry.base,
            length: entry.length,
            kind: map_e820_kind(entry.kind),
        };
        count += 1;
    }
    if count == 0 {
        return Err(MemNormError::Empty);
    }
    let (mapped, count) = sort_and_check(&mapped, count)?;
    let mut resplit = [zero_region(); MEMORY_MAP_MAX_ENTRIES];
    let mut resplit_count = 0usize;
    for region in &mapped[..count] {
        let end = match region.base.checked_add(region.length) {
            Some(end) => end,
            None => return Err(MemNormError::Overflow),
        };
        let mut cursor = region.base;
        while cursor < end {
            if resplit_count == MEMORY_MAP_MAX_ENTRIES {
                return Err(MemNormError::Overflow);
            }
            let piece_end = split_point(cursor, end);
            let kind = if cursor < FIRST_MEGABYTE {
                reservation_kind(cursor, region.kind)
            } else {
                region.kind
            };
            resplit[resplit_count] = MemoryRegion {
                base: cursor,
                length: piece_end - cursor,
                kind,
            };
            resplit_count += 1;
            cursor = piece_end;
        }
    }
    let (resplit, resplit_count) = sort_and_check(&resplit, resplit_count)?;
    let mut merged = [zero_region(); MEMORY_MAP_MAX_ENTRIES];
    let mut merged_count = 0usize;
    for region in &resplit[..resplit_count] {
        if let Some(last) = merged[..merged_count].last_mut() {
            if last.kind == region.kind && last.base + last.length == region.base {
                last.length += region.length;
                continue;
            }
        }
        if merged_count == MEMORY_MAP_MAX_ENTRIES {
            return Err(MemNormError::Overflow);
        }
        merged[merged_count] = *region;
        merged_count += 1;
    }
    let mut final_regions = merged;
    let mut final_count = merged_count;
    if let Some((carve_base, carve_length)) = kernel_carve_out {
        let carve_end = carve_base + carve_length;
        let mut carved = [zero_region(); MEMORY_MAP_MAX_ENTRIES];
        let mut carved_count = 0usize;
        for region in &final_regions[..final_count] {
            let end = region.base + region.length;
            let overlap_start = region.base.max(carve_base);
            let overlap_end = end.min(carve_end);
            if overlap_start >= overlap_end {
                push(&mut carved, &mut carved_count, *region)?;
                continue;
            }
            if region.base < overlap_start {
                push(
                    &mut carved,
                    &mut carved_count,
                    MemoryRegion {
                        base: region.base,
                        length: overlap_start - region.base,
                        kind: region.kind,
                    },
                )?;
            }
            push(
                &mut carved,
                &mut carved_count,
                MemoryRegion {
                    base: overlap_start,
                    length: overlap_end - overlap_start,
                    kind: MemoryKind::KernelAndModules,
                },
            )?;
            if overlap_end < end {
                push(
                    &mut carved,
                    &mut carved_count,
                    MemoryRegion {
                        base: overlap_end,
                        length: end - overlap_end,
                        kind: region.kind,
                    },
                )?;
            }
        }
        final_regions = carved;
        final_count = carved_count;
    }
    if final_count > out.len() {
        return Err(MemNormError::Overflow);
    }
    out[..final_count].copy_from_slice(&final_regions[..final_count]);
    Ok(final_count)
}

pub fn region_sum(regions: &[MemoryRegion]) -> u64 {
    regions.iter().map(|region| region.length).sum()
}

pub fn usable_bytes(regions: &[MemoryRegion]) -> u64 {
    regions
        .iter()
        .filter(|region| region.kind == MemoryKind::Usable)
        .map(|region| region.length)
        .sum()
}

pub fn top_of_usable(regions: &[MemoryRegion]) -> u64 {
    regions
        .iter()
        .filter(|region| region.kind == MemoryKind::Usable)
        .map(|region| region.base + region.length)
        .max()
        .unwrap_or(0)
}

fn map_e820_kind(kind: u32) -> MemoryKind {
    match kind {
        1 => MemoryKind::Usable,
        _ => MemoryKind::Reserved,
    }
}

fn reservation_kind(address: u64, detected: MemoryKind) -> MemoryKind {
    RESERVATIONS
        .iter()
        .find(|(start, end, _)| *start <= address && address < *end)
        .map(|(_, _, kind)| *kind)
        .unwrap_or(detected)
}

fn split_point(cursor: u64, end: u64) -> u64 {
    let mut candidate = end;
    if cursor < FIRST_MEGABYTE {
        for (start, stop, _) in RESERVATIONS {
            for boundary in [*start, *stop] {
                if cursor < boundary && boundary < candidate {
                    candidate = boundary;
                }
            }
        }
    }
    candidate
}

fn zero_region() -> MemoryRegion {
    MemoryRegion {
        base: 0,
        length: 0,
        kind: MemoryKind::Reserved,
    }
}

fn push(
    out: &mut [MemoryRegion],
    count: &mut usize,
    region: MemoryRegion,
) -> Result<(), MemNormError> {
    if *count == MEMORY_MAP_MAX_ENTRIES {
        return Err(MemNormError::Overflow);
    }
    out[*count] = region;
    *count += 1;
    Ok(())
}

fn sort_and_check(
    regions: &[MemoryRegion; MEMORY_MAP_MAX_ENTRIES],
    count: usize,
) -> Result<([MemoryRegion; MEMORY_MAP_MAX_ENTRIES], usize), MemNormError> {
    let mut sorted = *regions;
    for i in 1..count {
        let mut j = i;
        while j > 0 && sorted[j - 1].base > sorted[j].base {
            sorted.swap(j - 1, j);
            j -= 1;
        }
    }
    for window in sorted[..count].windows(2) {
        if window[0].base + window[0].length > window[1].base {
            return Err(MemNormError::RawOverlap);
        }
    }
    Ok((sorted, count))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out_buf() -> [MemoryRegion; MEMORY_MAP_MAX_ENTRIES] {
        [zero_region(); MEMORY_MAP_MAX_ENTRIES]
    }

    fn run(raw: &[RawEntry]) -> Vec<MemoryRegion> {
        let mut out = out_buf();
        let count = normalize(raw, None, &mut out).expect("normalize");
        out[..count].to_vec()
    }

    fn region(base: u64, length: u64, kind: MemoryKind) -> MemoryRegion {
        MemoryRegion { base, length, kind }
    }

    #[test]
    fn empty_raw_input_is_an_error() {
        assert_eq!(
            normalize(&[], None, &mut out_buf()),
            Err(MemNormError::Empty)
        );
    }

    #[test]
    fn raw_entries_may_arrive_out_of_order() {
        let raw = [
            RawEntry::usable(FIRST_MEGABYTE, 0x10_0000),
            RawEntry::usable(0, FIRST_MEGABYTE),
        ];
        let map = run(&raw);
        assert!(map.windows(2).all(|w| w[0].base + w[0].length <= w[1].base));
        assert_eq!(map.first().unwrap().base, 0);
    }

    #[test]
    fn overlapping_raw_entries_are_refused() {
        let raw = [
            RawEntry::usable(0, 0x20_0000),
            RawEntry::usable(0x10_0000, 0x10_0000),
        ];
        assert_eq!(
            normalize(&raw, None, &mut out_buf()),
            Err(MemNormError::RawOverlap)
        );
    }

    #[test]
    fn the_first_megabyte_is_resplit_exactly_per_the_table() {
        let raw = [RawEntry::usable(0, 0x0009_FC00)];
        let map = run(&raw);
        let expected = [
            region(0x0000_0000, 0x0000_0500, MemoryKind::Reserved),
            region(0x0000_0500, 0x0002_FB00, MemoryKind::BootloaderReclaimable),
            region(0x0003_0000, 0x0000_1000, MemoryKind::Reserved),
            region(0x0003_1000, 0x0000_F000, MemoryKind::Usable),
            region(0x0004_0000, 0x0003_C000, MemoryKind::BootloaderReclaimable),
            region(0x0007_C000, 0x0002_3C00, MemoryKind::Reserved),
        ];
        assert_eq!(map, expected);
    }

    #[test]
    fn a_bios_claiming_the_video_window_usable_is_overridden() {
        let raw = [
            RawEntry::usable(0, 0x0009_FC00),
            RawEntry {
                base: 0x000A_0000,
                length: 0x0006_0000,
                kind: 1,
            },
            RawEntry::usable(FIRST_MEGABYTE, 0x10_0000),
        ];
        let map = run(&raw);
        let video = map
            .iter()
            .find(|r| r.base <= 0x000B_8000 && 0x000B_8000 < r.base + r.length)
            .unwrap();
        assert_eq!(video.kind, MemoryKind::Reserved);
    }

    #[test]
    fn adjacent_same_kind_pieces_merge_and_the_table_overrides_kinds() {
        let raw = [
            RawEntry::usable(0, 0x0004_0000),
            RawEntry {
                base: 0x0004_0000,
                length: 0x0003_C000,
                kind: 2,
            },
            RawEntry::usable(FIRST_MEGABYTE, 0x10_0000),
        ];
        let map = run(&raw);
        let reclaimable = map
            .iter()
            .filter(|r| r.kind == MemoryKind::BootloaderReclaimable)
            .count();
        assert_eq!(reclaimable, 2);
    }

    #[test]
    fn unreported_holes_are_never_invented() {
        let raw = [
            RawEntry::usable(0, 0x0009_FC00),
            RawEntry {
                base: 0x000F_0000,
                length: 0x0001_0000,
                kind: 2,
            },
        ];
        let map = run(&raw);
        assert_eq!(region_sum(&map), 0x0009_FC00 + 0x0001_0000);
        assert!(
            map.iter()
                .all(|r| !(r.base < 0x000A_0000 && 0x000E_FFFF < r.base + r.length))
        );
    }

    #[test]
    fn a_kernel_carve_out_relabels_covered_usable_ram() {
        let raw = [
            RawEntry::usable(0, FIRST_MEGABYTE),
            RawEntry::usable(FIRST_MEGABYTE, 0x0080_0000),
        ];
        let mut out = out_buf();
        let count = normalize(&raw, Some((0x11_0000, 0x20_0000)), &mut out).unwrap();
        let map = &out[..count];
        let carved = map
            .iter()
            .find(|r| r.kind == MemoryKind::KernelAndModules)
            .expect("carve-out present");
        assert_eq!(carved.base, 0x11_0000);
        assert_eq!(carved.length, 0x0020_0000);
        assert_eq!(region_sum(map), FIRST_MEGABYTE + 0x0080_0000);
    }

    #[test]
    fn a_carve_out_splitting_a_region_keeps_the_sum() {
        let raw = [RawEntry::usable(0, FIRST_MEGABYTE)];
        let mut out = out_buf();
        let count = normalize(&raw, Some((0x0004_0000, 0x0001_0000)), &mut out).unwrap();
        let map = &out[..count];
        assert_eq!(region_sum(map), FIRST_MEGABYTE);
        assert!(map.windows(2).all(|w| w[0].base + w[0].length <= w[1].base));
    }

    #[test]
    fn the_synthesized_e801_map_normalizes_to_a_clean_map() {
        let (raw, count) = synthesize_e801(15 * 1024, 0);
        let map = run(&raw[..count]);
        assert_eq!(region_sum(&map), 16 * FIRST_MEGABYTE);
        assert_eq!(usable_bytes(&map), 0x0000_F000 + 15 * FIRST_MEGABYTE);
    }

    #[test]
    fn e801_block_sentinel_is_reinterpreted_as_64_kib_blocks() {
        let (raw, count) = synthesize_e801(E801_BLOCK_SENTINEL, 0);
        assert_eq!(raw[1].length, u64::from(E801_BLOCK_SENTINEL) * 64 * 1024);
        let map = run(&raw[..count]);
        assert_eq!(top_of_usable(&map), FIRST_MEGABYTE + 460_800 * 1024);
    }

    #[test]
    fn e801_above_16_mebibytes_lands_beyond_the_gap() {
        let (raw, count) = synthesize_e801(15 * 1024, 63 * 1024);
        let map = run(&raw[..count]);
        assert_eq!(top_of_usable(&map), SIXTEEN_MEGABYTES + 63 * 1024 * 1024);
    }

    #[test]
    fn the_e88_synthesizer_caps_at_sixty_four_mebibytes() {
        let (raw, count) = synthesize_e88(0xFFFF);
        assert_eq!(raw[1].length, E88_CAP_BYTES);
        let map = run(&raw[..count]);
        assert_eq!(top_of_usable(&map), FIRST_MEGABYTE + E88_CAP_BYTES);
    }

    #[test]
    fn overflow_past_the_map_capacity_refuses_instead_of_truncating() {
        let mut raw = [RawEntry::usable(0, 0); 3];
        raw[0] = RawEntry::usable(0, FIRST_MEGABYTE);
        raw[1] = RawEntry::usable(FIRST_MEGABYTE, 0x0010_0000);
        raw[2] = RawEntry {
            base: FIRST_MEGABYTE + 0x0010_0000,
            length: 0x0001_0000,
            kind: 2,
        };
        let mut out = out_buf();
        assert_eq!(normalize(&raw, None, &mut out), Ok(8));

        let many: Vec<RawEntry> = (0..MEMORY_MAP_MAX_ENTRIES + 1)
            .map(|i| RawEntry::usable((i as u64) * 0x0100_0000, 0x0010_0000))
            .collect();
        assert_eq!(
            normalize(&many, None, &mut out),
            Err(MemNormError::Overflow)
        );
    }

    #[test]
    fn zero_length_raw_entries_are_dropped_not_fatal() {
        let raw = [RawEntry::usable(0, 0), RawEntry::usable(0, FIRST_MEGABYTE)];
        let map = run(&raw);
        assert!(!map.is_empty());
    }

    #[test]
    fn output_is_sorted_non_overlapping_and_free_of_zero_length() {
        let raw = [
            RawEntry {
                base: FIRST_MEGABYTE,
                length: 0x0100_0000,
                kind: 2,
            },
            RawEntry::usable(0, 0x0009_FC00),
            RawEntry {
                base: 0x0009_FC00,
                length: 0x400,
                kind: 3,
            },
        ];
        let map = run(&raw);
        assert!(map.iter().all(|r| r.length > 0));
        assert!(map.windows(2).all(|w| w[0].base + w[0].length <= w[1].base));
    }
}
