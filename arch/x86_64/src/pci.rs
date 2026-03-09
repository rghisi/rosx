use x86_64::instructions::port::Port;

const CONFIG_ADDRESS_PORT: u16 = 0xCF8;
const CONFIG_DATA_PORT: u16 = 0xCFC;

const VENDOR_ID_OFFSET: u8 = 0x00;
const BAR0_OFFSET: u8 = 0x10;
const IRQ_LINE_OFFSET: u8 = 0x3C;

pub struct PciDevice {
    pub vendor_id: u16,
    pub device_id: u16,
    pub bar0: u32,
    pub irq_line: u8,
}

fn config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | ((offset as u32) & 0xFC)
}

unsafe fn read_config_u32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let mut address_port: Port<u32> = Port::new(CONFIG_ADDRESS_PORT);
    let mut data_port: Port<u32> = Port::new(CONFIG_DATA_PORT);
    address_port.write(config_address(bus, device, function, offset));
    data_port.read()
}

unsafe fn read_config_u16(bus: u8, device: u8, function: u8, offset: u8) -> u16 {
    let dword = read_config_u32(bus, device, function, offset);
    let shift = (offset & 2) * 8;
    (dword >> shift) as u16
}

unsafe fn read_config_u8(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
    let dword = read_config_u32(bus, device, function, offset);
    let shift = (offset & 3) * 8;
    (dword >> shift) as u8
}

pub fn find_device(vendor_id: u16, device_id: u16) -> Option<PciDevice> {
    for bus in 0u8..=255 {
        for device in 0u8..32 {
            for function in 0u8..8 {
                let raw = unsafe { read_config_u16(bus, device, function, VENDOR_ID_OFFSET) };
                if raw == 0xFFFF {
                    continue;
                }
                let found_vendor = raw;
                let found_device =
                    unsafe { read_config_u16(bus, device, function, VENDOR_ID_OFFSET + 2) };

                if found_vendor == vendor_id && found_device == device_id {
                    let bar0 =
                        unsafe { read_config_u32(bus, device, function, BAR0_OFFSET) };
                    let irq_line =
                        unsafe { read_config_u8(bus, device, function, IRQ_LINE_OFFSET) };
                    return Some(PciDevice {
                        vendor_id: found_vendor,
                        device_id: found_device,
                        bar0,
                        irq_line,
                    });
                }
            }
        }
    }
    None
}
