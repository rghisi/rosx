use alloc::vec::Vec;
use pci_protocol::wire::{Slot, MAX_FUNCTIONS};

pub const CONFIG_ADDRESS_PORT: u16 = 0xCF8;
pub const CONFIG_DATA_PORT: u16 = 0xCFC;
pub const CONFIG_PORTS_END: u16 = 0xCFF;
pub const CONFIG_ENABLE_BIT: u32 = 0x8000_0000;

const OFFSET_VENDOR_DEVICE: u8 = 0x00;
const OFFSET_COMMAND: u8 = 0x04;
const OFFSET_REVISION_CLASS: u8 = 0x08;
const OFFSET_BAR0: u8 = 0x10;
const OFFSET_INTERRUPT_LINE: u8 = 0x3C;

const COMMAND_IO_SPACE_ENABLE: u32 = 0x0000_0001;
const DEVICE_ABSENT: u16 = 0xFFFF;
const DEVICE_SLOTS: u8 = 32;

pub trait ConfigBus {
    fn read_dword(&mut self, bus: u8, device: u8, function: u8, offset: u8) -> u32;
    fn write_dword(&mut self, bus: u8, device: u8, function: u8, offset: u8, value: u32);
}

pub fn config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    CONFIG_ENABLE_BIT
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | (offset as u32 & 0xFC)
}

fn read_slot(
    bus: u8,
    device: u8,
    function: u8,
    config: &mut impl ConfigBus,
) -> Slot {
    let vendor_device = config.read_dword(bus, device, function, OFFSET_VENDOR_DEVICE);
    let revision_class = config.read_dword(bus, device, function, OFFSET_REVISION_CLASS);
    Slot {
        bus,
        device,
        function,
        vendor_id: vendor_device as u16,
        device_id: (vendor_device >> 16) as u16,
        revision: revision_class as u8,
        bar0: config.read_dword(bus, device, function, OFFSET_BAR0),
        irq: config.read_dword(bus, device, function, OFFSET_INTERRUPT_LINE) as u8,
    }
}

pub fn scan_bus(bus: u8, config: &mut impl ConfigBus) -> Vec<Slot> {
    let mut slots = Vec::new();
    for device in 0..DEVICE_SLOTS {
        for function in 0..MAX_FUNCTIONS {
            let vendor_device = config.read_dword(bus, device, function, OFFSET_VENDOR_DEVICE);
            if vendor_device as u16 == DEVICE_ABSENT {
                continue;
            }
            slots.push(read_slot(bus, device, function, config));
        }
    }
    slots
}

pub fn find_slot(slots: &[Slot], vendor_id: u16, device_id: u16) -> Option<&Slot> {
    slots
        .iter()
        .find(|slot| slot.vendor_id == vendor_id && slot.device_id == device_id)
}

pub fn enable_io_space(
    slots: &mut Vec<Slot>,
    bus: u8,
    device: u8,
    function: u8,
    config: &mut impl ConfigBus,
) -> Option<Slot> {
    if config.read_dword(bus, device, function, OFFSET_VENDOR_DEVICE) as u16 == DEVICE_ABSENT {
        return None;
    }
    let command = config.read_dword(bus, device, function, OFFSET_COMMAND);
    config.write_dword(
        bus,
        device,
        function,
        OFFSET_COMMAND,
        command | COMMAND_IO_SPACE_ENABLE,
    );
    let enabled = read_slot(bus, device, function, config);
    if let Some(cached) = slots.iter_mut().find(|slot| {
        slot.bus == bus && slot.device == device && slot.function == function
    }) {
        *cached = enabled.clone();
    }
    Some(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    struct MockConfig {
        registers: Vec<((u8, u8, u8, u8), u32)>,
        writes: Vec<((u8, u8, u8, u8), u32)>,
    }

    impl MockConfig {
        fn new() -> MockConfig {
            MockConfig {
                registers: Vec::new(),
                writes: Vec::new(),
            }
        }

        fn with(mut self, bus: u8, device: u8, function: u8, offset: u8, value: u32) -> Self {
            self.registers
                .push(((bus, device, function, offset & 0xFC), value));
            self
        }

        fn rtl8029(bus: u8, device: u8) -> MockConfig {
            MockConfig::new()
                .with(bus, device, 0, 0x00, 0x8029_10EC)
                .with(bus, device, 0, 0x04, 0x0000_0000)
                .with(bus, device, 0, 0x08, 0x0200_0000)
                .with(bus, device, 0, 0x10, 0x0000_D001)
                .with(bus, device, 0, 0x3C, 0x0000_000A)
        }
    }

    impl ConfigBus for MockConfig {
        fn read_dword(&mut self, bus: u8, device: u8, function: u8, offset: u8) -> u32 {
            let key = (bus, device, function, offset & 0xFC);
            self.registers
                .iter()
                .find(|(at, _)| *at == key)
                .map(|(_, value)| *value)
                .unwrap_or(0xFFFF_FFFF)
        }

        fn write_dword(&mut self, bus: u8, device: u8, function: u8, offset: u8, value: u32) {
            let key = (bus, device, function, offset & 0xFC);
            self.writes.push((key, value));
            if let Some((_, stored)) = self.registers.iter_mut().find(|(at, _)| *at == key) {
                *stored = value;
            }
        }
    }

    #[test]
    fn address_formula_sets_enable_and_lays_out_bus_device_function_offset() {
        assert_eq!(config_address(0, 0, 0, 0), 0x8000_0000);
        assert_eq!(config_address(1, 4, 2, 0x3C), 0x8001_223C);
        assert_eq!(config_address(255, 31, 7, 0xFC), 0x80FF_FFFC);
    }

    #[test]
    fn scan_collects_present_functions_and_skips_absent_ones() {
        let mut config = MockConfig::rtl8029(0, 4);
        let slots = scan_bus(0, &mut config);
        assert_eq!(slots.len(), 1);
        assert_eq!(
            slots[0],
            Slot {
                bus: 0,
                device: 4,
                function: 0,
                vendor_id: 0x10EC,
                device_id: 0x8029,
                revision: 0x00,
                bar0: 0xD001,
                irq: 10,
            }
        );
    }

    #[test]
    fn find_slot_matches_vendor_and_device() {
        let mut config = MockConfig::rtl8029(0, 4);
        let slots = scan_bus(0, &mut config);
        assert!(find_slot(&slots, 0x10EC, 0x8029).is_some());
        assert!(find_slot(&slots, 0x10EC, 0x8028).is_none());
        assert!(find_slot(&slots, 0x8086, 0x1234).is_none());
    }

    #[test]
    fn enable_sets_io_space_bit_and_refreshes_the_cache() {
        let mut config = MockConfig::rtl8029(0, 4);
        let mut slots = scan_bus(0, &mut config);
        let enabled = enable_io_space(&mut slots, 0, 4, 0, &mut config).expect("device present");
        assert_eq!(enabled.bar0, 0xD001);
        assert_eq!(config.writes[0].1, COMMAND_IO_SPACE_ENABLE);
        assert_eq!(slots[0], enabled);
    }

    #[test]
    fn enable_refuses_an_absent_slot() {
        let mut config = MockConfig::new();
        let mut slots = Vec::new();
        assert_eq!(enable_io_space(&mut slots, 0, 9, 0, &mut config), None);
        assert!(config.writes.is_empty());
    }

    #[test]
    fn empty_bus_scans_to_nothing() {
        let mut config = MockConfig::new();
        assert_eq!(scan_bus(0, &mut config), vec![]);
    }
}
