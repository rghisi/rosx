#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod mbr;
pub mod rule;

pub use mbr::{
    Entry, Table, TableState, ENTRY_BYTES, ENTRY_COUNT, SECTOR_BYTES, SIGNATURE, Slot, TABLE_OFFSET,
    parse_mbr,
};
pub use rule::{Exposure, Slice, exposure, is_supported};
