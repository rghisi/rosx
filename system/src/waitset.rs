#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WaitEvent {
    pub child: u32,
    pub tag: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_event_is_eight_bytes() {
        assert_eq!(core::mem::size_of::<WaitEvent>(), 8);
    }

    #[test]
    fn wait_event_field_offsets_match_the_treaty() {
        assert_eq!(core::mem::offset_of!(WaitEvent, child), 0);
        assert_eq!(core::mem::offset_of!(WaitEvent, tag), 4);
    }
}
