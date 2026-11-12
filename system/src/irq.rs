use crate::ipc::MESSAGE_PAYLOAD_BYTES;

pub const MAX_IRQ: u32 = 15;

pub const SOURCE_NAMES: [&str; (MAX_IRQ + 1) as usize] = [
    "IRQ0", "IRQ1", "IRQ2", "IRQ3", "IRQ4", "IRQ5", "IRQ6", "IRQ7", "IRQ8", "IRQ9", "IRQ10",
    "IRQ11", "IRQ12", "IRQ13", "IRQ14", "IRQ15",
];

const TAG_IRQ_EVENT: u8 = 1;

pub fn is_subscribable(irq: u32) -> bool {
    matches!(irq, 3..=7 | 9..=15)
}

pub fn encode_irq_event(irq: u32) -> [u8; MESSAGE_PAYLOAD_BYTES] {
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    data[0] = TAG_IRQ_EVENT;
    data[1] = irq as u8;
    data
}

pub fn event_of(data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> Option<u32> {
    if data[0] != TAG_IRQ_EVENT || u32::from(data[1]) > MAX_IRQ {
        return None;
    }
    Some(u32::from(data[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_names_cover_every_irq_with_its_decimal_name() {
        assert_eq!(SOURCE_NAMES.len(), (MAX_IRQ + 1) as usize);
        assert_eq!(SOURCE_NAMES[0], "IRQ0");
        assert_eq!(SOURCE_NAMES[9], "IRQ9");
        assert_eq!(SOURCE_NAMES[15], "IRQ15");
    }

    #[test]
    fn reserved_lines_are_not_subscribable() {
        for irq in [0, 1, 2, 8] {
            assert!(!is_subscribable(irq), "irq {irq} must stay reserved");
        }
        for irq in [3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15] {
            assert!(is_subscribable(irq), "irq {irq} must be subscribable");
        }
        assert!(!is_subscribable(MAX_IRQ + 1));
    }

    #[test]
    fn event_frames_round_trip_through_event_of() {
        for irq in 0..=MAX_IRQ {
            assert_eq!(event_of(&encode_irq_event(irq)), Some(irq));
        }
    }

    #[test]
    fn foreign_frames_decode_as_no_event() {
        let zeroed = [0u8; MESSAGE_PAYLOAD_BYTES];
        assert_eq!(event_of(&zeroed), None);

        let mut unknown_tag = encode_irq_event(9);
        unknown_tag[0] = 200;
        assert_eq!(event_of(&unknown_tag), None);

        let mut out_of_range = [0u8; MESSAGE_PAYLOAD_BYTES];
        out_of_range[0] = TAG_IRQ_EVENT;
        out_of_range[1] = 16;
        assert_eq!(event_of(&out_of_range), None);
    }
}
