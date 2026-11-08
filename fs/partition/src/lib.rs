#![cfg_attr(not(test), no_std)]

pub mod mbr;

pub use mbr::{
    Entry, Table, TableState, ENTRY_BYTES, ENTRY_COUNT, SECTOR_BYTES, SIGNATURE, Slot, TABLE_OFFSET,
    parse_mbr,
};
