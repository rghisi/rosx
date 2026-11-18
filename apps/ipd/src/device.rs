use alloc::vec::Vec;
use net_protocol::ethernet::MAC_BYTES;

pub trait FrameDevice {
    fn mac(&self) -> [u8; MAC_BYTES];
    fn transmit(&mut self, frame: &[u8]);
}

pub struct MockDevice {
    mac: [u8; MAC_BYTES],
    sent: Vec<Vec<u8>>,
}

impl MockDevice {
    pub fn new(mac: [u8; MAC_BYTES]) -> MockDevice {
        MockDevice {
            mac,
            sent: Vec::new(),
        }
    }

    pub fn sent(&self) -> &[Vec<u8>] {
        &self.sent
    }

    pub fn sent_count(&self) -> usize {
        self.sent.len()
    }
}

impl FrameDevice for MockDevice {
    fn mac(&self) -> [u8; MAC_BYTES] {
        self.mac
    }

    fn transmit(&mut self, frame: &[u8]) {
        self.sent.push(frame.to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: [u8; MAC_BYTES] = [0x02, 0x52, 0x4e, 0x45, 0x32, 0x07];

    #[test]
    fn mock_records_transmits_in_order() {
        let mut device = MockDevice::new(MAC);
        assert_eq!(device.mac(), MAC);
        assert_eq!(device.sent_count(), 0);
        device.transmit(&[1, 2, 3]);
        device.transmit(&[4]);
        assert_eq!(device.sent(), &alloc::vec![alloc::vec![1, 2, 3], alloc::vec![4]]);
        assert_eq!(device.sent_count(), 2);
    }
}
