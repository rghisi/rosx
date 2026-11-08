//! rxpart reads master boot records and returns verdicts: it parses the 512-byte
//! master boot record plus the disk's sector count and decides how the disk should
//! be addressed. Pure data in, verdicts out — it never touches a device, knows no
//! service or naming scheme, and its parse allocates nothing.
//!
//! Vocabulary: a controller hosts disks; a disk carries at most one master boot
//! record whose in-use slots (0..=3) address primary partitions; volume names a
//! mounted filesystem and is reserved for rxfs — never a partition.
//!
//! The curation rule (`exposure`): a disk is addressed whole only when it has no
//! readable, supported partition table. A valid table with at least one supported
//! entry exposes one slice per supported entry at its true slot index. No
//! signature, an empty table, or a corrupt table exposes the whole disk with the
//! state reported honestly — expose-on-corrupt, because this ecosystem has no
//! table-writing tool yet and a parser must never brick a disk. A valid table
//! whose entries are all extended/logical containers (0x05, 0x0F, 0x85) or a GPT
//! protective entry (0xEE) also exposes the whole disk: v1 recognizes those
//! containers but cannot address their contents.
//!
//! Scope: MBR primary partitions only; CHS fields and the boot indicator are
//! parsed but ignored; extended/logical chain walking and GPT are future work.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod mbr;
pub mod rule;

pub use mbr::{
    Entry, Table, TableState, ENTRY_BYTES, ENTRY_COUNT, SECTOR_BYTES, SIGNATURE, Slot, TABLE_OFFSET,
    encode_mbr, parse_mbr,
};
pub use rule::{Exposure, Slice, exposure, is_supported};
