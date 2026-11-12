use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const SERVICE_NAME: &str = "PCI";pub const FRAME_BYTES: usize = 60;
pub const SLOT_STRIDE_BYTES: usize = 16;
pub const MAX_FUNCTIONS: u8 = 8;

pub const FIND: u8 = 0x01;
pub const LIST: u8 = 0x02;
pub const ENABLE: u8 = 0x03;

pub const REPLY_SLOT: u8 = 0x10;
pub const REPLY_MISSING: u8 = 0x11;
pub const REPLY_ERROR: u8 = 0x12;
pub const REPLY_LIST: u8 = 0x13;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum PciError {
    Protocol = 255,
}

impl PciError {
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(byte: u8) -> PciError {
        match byte {
            255 => PciError::Protocol,
            _ => PciError::Protocol,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Slot {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub revision: u8,
    pub bar0: u32,
    pub irq: u8,
}

impl Slot {
    pub fn location(&self) -> Option<String> {
        if self.device > 31 || self.function >= MAX_FUNCTIONS {
            return None;
        }
        Some(format!(
            "{:02x}:{:02x}.{} rev {:02x} bar0 {:#06x} irq {}",
            self.bus, self.device, self.function, self.revision, self.bar0, self.irq
        ))
    }
}

fn valid_location(_bus: u8, device: u8, function: u8) -> bool {
    device <= 31 && function < MAX_FUNCTIONS
}

fn valid_slot(slot: &Slot) -> bool {
    valid_location(slot.bus, slot.device, slot.function)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Request {
    Find { vendor_id: u16, device_id: u16 },
    List { bus: u8 },
    Enable { bus: u8, device: u8, function: u8 },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Reply {
    Slot(Slot),
    Missing,
    Err { error: PciError },
    List { token: u32, entries: u8 },
}

fn encode_slot_bytes(entry: &mut [u8; SLOT_STRIDE_BYTES], slot: &Slot) {
    entry[0..2].copy_from_slice(&slot.vendor_id.to_le_bytes());
    entry[2..4].copy_from_slice(&slot.device_id.to_le_bytes());
    entry[4] = slot.bus;
    entry[5] = slot.device;
    entry[6] = slot.function;
    entry[7] = slot.revision;
    entry[8..12].copy_from_slice(&slot.bar0.to_le_bytes());
    entry[12] = slot.irq;
    entry[13..SLOT_STRIDE_BYTES].fill(0);
}

fn decode_slot_bytes(entry: &[u8; SLOT_STRIDE_BYTES]) -> Result<Slot, PciError> {
    if entry[13..SLOT_STRIDE_BYTES].iter().any(|byte| *byte != 0) {
        return Err(PciError::Protocol);
    }
    let slot = Slot {
        bus: entry[4],
        device: entry[5],
        function: entry[6],
        vendor_id: u16::from_le_bytes(entry[0..2].try_into().unwrap()),
        device_id: u16::from_le_bytes(entry[2..4].try_into().unwrap()),
        revision: entry[7],
        bar0: u32::from_le_bytes(entry[8..12].try_into().unwrap()),
        irq: entry[12],
    };
    if !valid_slot(&slot) {
        return Err(PciError::Protocol);
    }
    Ok(slot)
}

pub fn encode_slots_table(slots: &[Slot]) -> Vec<u8> {
    let mut blob = alloc::vec![0u8; slots.len() * SLOT_STRIDE_BYTES];
    for (index, slot) in slots.iter().enumerate() {
        let at = index * SLOT_STRIDE_BYTES;
        let entry = &mut blob[at..at + SLOT_STRIDE_BYTES];
        entry.copy_from_slice(&{
            let mut encoded = [0u8; SLOT_STRIDE_BYTES];
            encode_slot_bytes(&mut encoded, slot);
            encoded
        });
    }
    blob
}

pub fn decode_slots_table(blob: &[u8]) -> Result<Vec<Slot>, PciError> {
    if !blob.len().is_multiple_of(SLOT_STRIDE_BYTES) {
        return Err(PciError::Protocol);
    }
    let mut slots = Vec::with_capacity(blob.len() / SLOT_STRIDE_BYTES);
    for chunk in blob.chunks_exact(SLOT_STRIDE_BYTES) {
        slots.push(decode_slot_bytes(chunk.try_into().unwrap())?);
    }
    Ok(slots)
}

impl Request {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], PciError> {
        let mut frame = [0u8; FRAME_BYTES];
        match self {
            Request::Find {
                vendor_id,
                device_id,
            } => {
                frame[0] = FIND;
                frame[1..3].copy_from_slice(&vendor_id.to_le_bytes());
                frame[3..5].copy_from_slice(&device_id.to_le_bytes());
            }
            Request::List { bus } => {
                frame[0] = LIST;
                frame[1] = *bus;
            }
            Request::Enable {
                bus,
                device,
                function,
            } => {
                if !valid_location(*bus, *device, *function) {
                    return Err(PciError::Protocol);
                }
                frame[0] = ENABLE;
                frame[1] = *bus;
                frame[2] = *device;
                frame[3] = *function;
            }
        }
        Ok(frame)
    }

    pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Request, PciError> {
        match frame[0] {
            FIND => {
                zero_tail(frame, 5)?;
                Ok(Request::Find {
                    vendor_id: u16::from_le_bytes(frame[1..3].try_into().unwrap()),
                    device_id: u16::from_le_bytes(frame[3..5].try_into().unwrap()),
                })
            }
            LIST => {
                zero_tail(frame, 2)?;
                Ok(Request::List { bus: frame[1] })
            }
            ENABLE => {
                zero_tail(frame, 4)?;
                if !valid_location(frame[1], frame[2], frame[3]) {
                    return Err(PciError::Protocol);
                }
                Ok(Request::Enable {
                    bus: frame[1],
                    device: frame[2],
                    function: frame[3],
                })
            }
            _ => Err(PciError::Protocol),
        }
    }
}

impl Reply {
    pub fn encode(&self) -> Result<[u8; FRAME_BYTES], PciError> {
        let mut frame = [0u8; FRAME_BYTES];
        match self {
            Reply::Slot(slot) => {
                if !valid_slot(slot) {
                    return Err(PciError::Protocol);
                }
                frame[0] = REPLY_SLOT;
                frame[1..1 + SLOT_STRIDE_BYTES]
                    .copy_from_slice(&{
                        let mut encoded = [0u8; SLOT_STRIDE_BYTES];
                        encode_slot_bytes(&mut encoded, slot);
                        encoded
                    });
            }
            Reply::Missing => frame[0] = REPLY_MISSING,
            Reply::Err { error } => {
                frame[0] = REPLY_ERROR;
                frame[1] = error.to_byte();
            }
            Reply::List { token, entries } => {
                frame[0] = REPLY_LIST;
                frame[1..5].copy_from_slice(&token.to_le_bytes());
                frame[5] = *entries;
            }
        }
        Ok(frame)
    }

    pub fn decode(frame: &[u8; FRAME_BYTES]) -> Result<Reply, PciError> {
        match frame[0] {
            REPLY_SLOT => {
                zero_tail(frame, 1 + SLOT_STRIDE_BYTES)?;
                let entry: &[u8; SLOT_STRIDE_BYTES] =
                    frame[1..1 + SLOT_STRIDE_BYTES].try_into().unwrap();
                Ok(Reply::Slot(decode_slot_bytes(entry)?))
            }
            REPLY_MISSING => {
                zero_tail(frame, 1)?;
                Ok(Reply::Missing)
            }
            REPLY_ERROR => {
                zero_tail(frame, 2)?;
                Ok(Reply::Err {
                    error: PciError::from_byte(frame[1]),
                })
            }
            REPLY_LIST => {
                zero_tail(frame, 6)?;
                Ok(Reply::List {
                    token: u32::from_le_bytes(frame[1..5].try_into().unwrap()),
                    entries: frame[5],
                })
            }
            _ => Err(PciError::Protocol),
        }
    }
}

fn zero_tail(frame: &[u8; FRAME_BYTES], from: usize) -> Result<(), PciError> {
    if frame[from..].iter().any(|byte| *byte != 0) {
        Err(PciError::Protocol)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot() -> Slot {
        Slot {
            bus: 0,
            device: 4,
            function: 0,
            vendor_id: 0x10EC,
            device_id: 0x8029,
            revision: 0,
            bar0: 0xD000 | 1,
            irq: 10,
        }
    }

    #[test]
    fn tag_constants_are_pinned_and_disjoint() {
        assert_eq!(FIND, 0x01);
        assert_eq!(LIST, 0x02);
        assert_eq!(ENABLE, 0x03);
        assert_eq!(REPLY_SLOT, 0x10);
        assert_eq!(REPLY_MISSING, 0x11);
        assert_eq!(REPLY_ERROR, 0x12);
        assert_eq!(REPLY_LIST, 0x13);
    }

    #[test]
    fn find_frame_pins_vendor_and_device_offsets() {
        let frame = Request::Find {
            vendor_id: 0x10EC,
            device_id: 0x8029,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], FIND);
        assert_eq!(&frame[1..3], &[0xEC, 0x10]);
        assert_eq!(&frame[3..5], &[0x29, 0x80]);
        assert!(frame[5..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn enable_frame_pins_the_slot_location_offsets() {
        let frame = Request::Enable {
            bus: 0,
            device: 4,
            function: 1,
        }
        .encode()
        .unwrap();
        assert_eq!(frame[0], ENABLE);
        assert_eq!(&frame[1..4], &[0, 4, 1]);
        assert!(frame[4..].iter().all(|byte| *byte == 0));

        let mut dirty = frame;
        dirty[4] = 1;
        assert_eq!(Request::decode(&dirty), Err(PciError::Protocol));

        for location in [(0u8, 32u8, 0u8), (0, 4, 8)] {
            assert_eq!(
                Request::Enable {
                    bus: location.0,
                    device: location.1,
                    function: location.2,
                }
                .encode(),
                Err(PciError::Protocol)
            );
            let mut smuggled = frame;
            smuggled[2] = location.1;
            smuggled[3] = location.2;
            assert_eq!(Request::decode(&smuggled), Err(PciError::Protocol));
        }
    }

    #[test]
    fn request_roundtrips_and_rejects_dirty_frames() {
        for request in [
            Request::Find {
                vendor_id: 0,
                device_id: 0xFFFF,
            },
            Request::Find {
                vendor_id: 0x8086,
                device_id: 0x1234,
            },
            Request::List { bus: 0 },
            Request::List { bus: 255 },
            Request::Enable {
                bus: 0,
                device: 4,
                function: 0,
            },
        ] {
            assert_eq!(Request::decode(&request.encode().unwrap()), Ok(request));
        }
        for op in [0u8, 4, 0x10, 0xFF] {
            let mut frame = Request::List { bus: 0 }.encode().unwrap();
            frame[0] = op;
            assert_eq!(Request::decode(&frame), Err(PciError::Protocol));
        }
        let mut dirty = Request::Find {
            vendor_id: 1,
            device_id: 2,
        }
        .encode()
        .unwrap();
        dirty[5] = 1;
        assert_eq!(Request::decode(&dirty), Err(PciError::Protocol));
    }

    #[test]
    fn slot_reply_pins_field_offsets() {
        let frame = Reply::Slot(slot()).encode().unwrap();
        assert_eq!(frame[0], REPLY_SLOT);
        assert_eq!(&frame[1..3], &[0xEC, 0x10]);
        assert_eq!(&frame[3..5], &[0x29, 0x80]);
        assert_eq!(frame[5], 0);
        assert_eq!(frame[6], 4);
        assert_eq!(frame[7], 0);
        assert_eq!(frame[8], 0);
        assert_eq!(&frame[9..13], &0xD001u32.to_le_bytes());
        assert_eq!(frame[13], 10);
        assert!(frame[14..17].iter().all(|byte| *byte == 0));
        assert!(frame[17..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn reply_all_variants_roundtrip() {
        let replies = [
            Reply::Slot(slot()),
            Reply::Slot(Slot {
                bus: 255,
                device: 31,
                function: 7,
                vendor_id: u16::MAX,
                device_id: u16::MAX,
                revision: u8::MAX,
                bar0: u32::MAX,
                irq: 255,
            }),
            Reply::Missing,
            Reply::Err {
                error: PciError::Protocol,
            },
            Reply::List {
                token: 0x0102_0304,
                entries: 0,
            },
            Reply::List {
                token: u32::MAX,
                entries: 255,
            },
        ];
        for reply in replies {
            assert_eq!(Reply::decode(&reply.encode().unwrap()), Ok(reply));
        }
    }

    #[test]
    fn slot_reply_refuses_device_beyond_the_addressable_bus() {
        assert_eq!(
            Reply::Slot(Slot {
                device: 32,
                ..slot()
            })
            .encode(),
            Err(PciError::Protocol)
        );
        assert_eq!(
            Reply::Slot(Slot {
                function: 8,
                ..slot()
            })
            .encode(),
            Err(PciError::Protocol)
        );
        let mut frame = Reply::Slot(slot()).encode().unwrap();
        frame[1 + 6] = 8;
        assert_eq!(Reply::decode(&frame), Err(PciError::Protocol));
    }

    #[test]
    fn reply_decode_rejects_unknown_tags_and_dirty_tails() {
        for tag in [0x00u8, 0x0F, 0x14, 0xFF] {
            let mut frame = [0u8; FRAME_BYTES];
            frame[0] = tag;
            assert_eq!(Reply::decode(&frame), Err(PciError::Protocol));
        }
        let mut dirty = Reply::Missing.encode().unwrap();
        dirty[1] = 1;
        assert_eq!(Reply::decode(&dirty), Err(PciError::Protocol));
        let mut dirty = Reply::List {
            token: 1,
            entries: 1,
        }
        .encode()
        .unwrap();
        dirty[6] = 1;
        assert_eq!(Reply::decode(&dirty), Err(PciError::Protocol));
    }

    #[test]
    fn slots_table_roundtrips_and_refuses_malformed_blobs() {
        let slots = [slot(), Slot { device: 5, ..slot() }];
        let blob = encode_slots_table(&slots);
        assert_eq!(blob.len(), 2 * SLOT_STRIDE_BYTES);
        assert_eq!(decode_slots_table(&blob), Ok(slots.to_vec()));
        assert_eq!(decode_slots_table(&[]), Ok(Vec::new()));
        assert_eq!(
            decode_slots_table(&[0u8; SLOT_STRIDE_BYTES - 1]),
            Err(PciError::Protocol)
        );

        let mut reserved_dirt = encode_slots_table(&slots[..1]);
        reserved_dirt[13] = 0x01;
        assert_eq!(decode_slots_table(&reserved_dirt), Err(PciError::Protocol));

        let mut bad_function = encode_slots_table(&slots[..1]);
        bad_function[6] = 8;
        assert_eq!(decode_slots_table(&bad_function), Err(PciError::Protocol));
    }

    #[test]
    fn slot_location_reports_the_pci_address() {
        assert_eq!(
            slot().location().as_deref(),
            Some("00:04.0 rev 00 bar0 0xd001 irq 10")
        );
        assert_eq!(Slot { device: 32, ..slot() }.location(), None);
    }

    fn seed_frames() -> Vec<[u8; FRAME_BYTES]> {
        let mut frames = Vec::new();
        for request in [
            Request::Find {
                vendor_id: 0x10EC,
                device_id: 0x8029,
            },
            Request::List { bus: 0 },
            Request::Enable {
                bus: 0,
                device: 4,
                function: 0,
            },
        ] {
            frames.push(request.encode().expect("seed request encodes"));
        }
        for reply in [
            Reply::Slot(slot()),
            Reply::Missing,
            Reply::Err {
                error: PciError::Protocol,
            },
            Reply::List {
                token: 9,
                entries: 3,
            },
        ] {
            frames.push(reply.encode().expect("seed reply encodes"));
        }
        frames
    }

    #[test]
    fn decoding_flips_never_panics() {
        for seed in seed_frames() {
            for byte in 0..FRAME_BYTES {
                for bit in 0..8 {
                    let mut mutated = seed;
                    mutated[byte] ^= 1 << bit;
                    let _ = Request::decode(&mutated);
                    let _ = Reply::decode(&mutated);
                }
            }
        }
    }
}
