use x86_64::instructions::port::Port;
use crate::pci::PciDevice;

const TX_BUF_SIZE: usize = 1536;
const TX_BUF_COUNT: usize = 4;
const RX_BUF_SIZE: usize = 8192 + 16 + 1500;

const REG_MAC: u16 = 0x00;
const REG_TSD0: u16 = 0x10;
const REG_TSAD0: u16 = 0x20;
const REG_RBSTART: u16 = 0x30;
const REG_CR: u16 = 0x37;
const REG_CAPR: u16 = 0x38;
const REG_IMR: u16 = 0x3C;
const REG_ISR: u16 = 0x3E;
const REG_RCR: u16 = 0x44;
const REG_CONFIG1: u16 = 0x52;

const CR_RESET: u8 = 0x10;
const CR_RX_ENABLE: u8 = 0x08;
const CR_TX_ENABLE: u8 = 0x04;
const CR_RX_BUF_EMPTY: u8 = 0x01;

const RCR_ACCEPT_PHYSICAL_MATCH: u32 = 1 << 1;
const RCR_ACCEPT_BROADCAST: u32 = 1 << 3;
const RCR_WRAP: u32 = 1 << 7;

const ISR_ROK: u16 = 0x0001;
const ISR_TOK: u16 = 0x0004;

static mut TX_BUFFERS: [[u8; TX_BUF_SIZE]; TX_BUF_COUNT] = [[0u8; TX_BUF_SIZE]; TX_BUF_COUNT];
static mut RX_BUFFER: [u8; RX_BUF_SIZE] = [0u8; RX_BUF_SIZE];

pub struct Rtl8139 {
    io_base: u16,
    tx_slot: usize,
    rx_offset: usize,
}

impl Rtl8139 {
    pub fn init(device: PciDevice) -> Self {
        let io_base = (device.bar0 & 0xFFFC) as u16;
        let mut nic = Rtl8139 { io_base, tx_slot: 0, rx_offset: 0 };

        unsafe {
            nic.write_u8(REG_CONFIG1, 0x00);

            nic.write_u8(REG_CR, CR_RESET);
            while nic.read_u8(REG_CR) & CR_RESET != 0 {}

            nic.write_u32(REG_RBSTART, RX_BUFFER.as_ptr() as u32);
            nic.write_u16(REG_CAPR, 0xFFF0);

            nic.write_u32(REG_RCR, RCR_ACCEPT_PHYSICAL_MATCH | RCR_ACCEPT_BROADCAST | RCR_WRAP);

            nic.write_u16(REG_IMR, ISR_ROK | ISR_TOK);

            nic.write_u8(REG_CR, CR_RX_ENABLE | CR_TX_ENABLE);
        }

        nic
    }

    pub fn mac_address(&mut self) -> [u8; 6] {
        let mut mac = [0u8; 6];
        for (i, byte) in mac.iter_mut().enumerate() {
            *byte = unsafe { self.read_u8(REG_MAC + i as u16) };
        }
        mac
    }

    pub fn send_packet(&mut self, data: &[u8]) {
        assert!(data.len() <= TX_BUF_SIZE);
        let slot = self.tx_slot;

        unsafe {
            TX_BUFFERS[slot][..data.len()].copy_from_slice(data);
            let tx_addr = TX_BUFFERS[slot].as_ptr() as u32;
            self.write_u32(REG_TSAD0 + slot as u16 * 4, tx_addr);
            self.write_u32(REG_TSD0 + slot as u16 * 4, data.len() as u32);
        }

        self.tx_slot = (slot + 1) % TX_BUF_COUNT;
    }

    pub fn receive_packet(&mut self, out: &mut [u8]) -> Option<usize> {
        let cr = unsafe { self.read_u8(REG_CR) };
        if cr & CR_RX_BUF_EMPTY != 0 {
            return None;
        }

        let rx = unsafe { &RX_BUFFER };
        let offset = self.rx_offset;

        let length = u16::from_le_bytes([rx[offset + 2], rx[offset + 3]]) as usize;
        let data_len = length.saturating_sub(4);

        let copy_len = data_len.min(out.len());
        out[..copy_len].copy_from_slice(&rx[offset + 4..offset + 4 + copy_len]);

        let next_offset = (offset + 4 + length + 3) & !3;
        self.rx_offset = next_offset % RX_BUF_SIZE;

        unsafe {
            self.write_u16(REG_CAPR, self.rx_offset as u16 - 16);
            self.write_u16(REG_ISR, ISR_ROK);
        }

        Some(copy_len)
    }

    unsafe fn read_u8(&mut self, reg: u16) -> u8 {
        Port::<u8>::new(self.io_base + reg).read()
    }

    unsafe fn read_u16(&mut self, reg: u16) -> u16 {
        Port::<u16>::new(self.io_base + reg).read()
    }

    unsafe fn write_u8(&mut self, reg: u16, val: u8) {
        Port::<u8>::new(self.io_base + reg).write(val);
    }

    unsafe fn write_u16(&mut self, reg: u16, val: u16) {
        Port::<u16>::new(self.io_base + reg).write(val);
    }

    unsafe fn write_u32(&mut self, reg: u16, val: u32) {
        Port::<u32>::new(self.io_base + reg).write(val);
    }
}
